use crate::{
    division_census::UnitInputs,
    province_unit_changes::{self, Changes, Counts},
};
use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_vec},
};
#[cfg(test)]
use collector_projection::table_conversion::table;
use engine_allocation::{advisory, copy_vec, owned_vec, reserve};
use engine_native::{
    Memory, carmy, cprovince, cprovince::KnownUnit, cprovince::UnitOrderRow, walk::Outcome,
};
use replay_data::{rows::ProvinceUnitOrderRow, rows::TableId, rows::tables};
#[cfg(test)]
use replay_frame::frame::TableView;
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceUnitOrder];

#[derive(Clone, Copy, Debug)]
pub struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    arena: usize,
}

collector_primitives::capacity_max!(Capacities { rows, arena });

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = cprovince::count_unit_order_with_land(memory, base, state, |_, _| Ok(()));
        let count = collector_extended_support::demand::rows(demand.outcome)?;
        collector_primitives::country_slots::raise(
            &mut self.rows,
            collector_primitives::country_slots::rows(count, 1),
        );
        collector_primitives::country_slots::raise(
            &mut self.arena,
            collector_primitives::country_slots::rows(demand.required, 1),
        );
        Ok(())
    }
}

#[derive(Default)]
struct Held {
    epoch: Option<u64>,
    header: [u8; 16],
    pointers: Vec<usize>,
    counts: Vec<[u32; 2]>,
    handles: Vec<u64>,
    land: Vec<(u32, u64)>,
}

impl Held {
    fn hold(
        &mut self,
        header: [u8; 16],
        pointers: &mut Vec<usize>,
        rows: &[UnitOrderRow],
        handles: &[u64],
        land: &[(u32, u64)],
    ) -> std::io::Result<()> {
        std::mem::swap(&mut self.pointers, pointers);
        self.counts.clear();
        advisory(|| reserve(&mut self.counts, rows.len()))?;
        self.counts.extend(
            rows.iter()
                .map(|row| [row.mixed[1] as u32, row.naval[1] as u32]),
        );
        self.hold_occupants(handles, land)?;
        self.header = header;
        Ok(())
    }

    fn hold_occupants(&mut self, handles: &[u64], land: &[(u32, u64)]) -> std::io::Result<()> {
        self.handles.clear();
        self.land.clear();
        advisory(|| {
            reserve(&mut self.handles, handles.len())?;
            reserve(&mut self.land, land.len())
        })?;
        self.handles.extend_from_slice(handles);
        self.land.extend_from_slice(land);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Hour {
    epoch: Option<u64>,
    carryable: bool,
    marks: u64,
}

pub struct ProvinceUnitStore {
    changes: &'static dyn Changes,
    held: Held,
    listed: Vec<u32>,
    slots: Vec<usize>,
    rows: Vec<UnitOrderRow>,
    handles: Vec<u64>,
    land: Vec<(u32, u64)>,
    bytes: usize,
}

impl ProvinceUnitStore {
    pub fn new(changes: &'static dyn Changes) -> Self {
        Self {
            changes,
            held: Held::default(),
            listed: Vec::new(),
            slots: Vec::new(),
            rows: Vec::new(),
            handles: Vec::new(),
            land: Vec::new(),
            bytes: 0,
        }
    }

    pub fn forget(&mut self) {
        self.held.epoch = None;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn holds(&self) -> bool {
        self.held.epoch.is_some()
    }

    fn begin(&mut self) -> Hour {
        self.listed.clear();
        let mut marks = 0;
        let mut kept = true;
        let listed = &mut self.listed;
        let epoch = self.changes.take(&mut |id| {
            marks += 1;
            match u32::try_from(id) {
                Ok(id) if advisory(|| reserve(listed, 1)).is_ok() => listed.push(id),
                _ => kept = false,
            }
        });
        let carryable = kept && epoch.is_some() && self.held.epoch == epoch;
        self.held.epoch = None;
        Hour {
            epoch,
            carryable,
            marks,
        }
    }

    fn measure(&mut self) {
        self.bytes = (self.held.pointers.capacity() + self.slots.capacity()) * 8
            + self.held.counts.capacity() * 8
            + (self.held.handles.capacity() + self.handles.capacity()) * 8
            + (self.held.land.capacity() + self.land.capacity())
                * std::mem::size_of::<(u32, u64)>()
            + self.listed.capacity() * 4
            + self.rows.capacity() * std::mem::size_of::<UnitOrderRow>();
    }
}

fn census_unit(known: &[(usize, KnownUnit)], unit: usize) -> Option<KnownUnit> {
    known
        .binary_search_by_key(&unit, |entry| entry.0)
        .ok()
        .map(|index| known[index].1)
}

pub struct Captured {
    rows: CaptureRows<cprovince::UnitOrderRow>,
    arena: Arena<u64>,
    terminal_failure: Option<&'static str>,
    land: Vec<(u32, u64)>,
    known: Vec<(usize, KnownUnit)>,
}
impl Captured {
    #[cfg(any(test, feature = "test-support"))]
    pub fn install_land_fixture(&mut self, land: Vec<(u32, u64)>, outcome: Outcome) {
        self.land = land;
        self.rows.outcome = outcome;
    }

    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, arena: 0 })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            arena: Arena::new(c.arena),
            terminal_failure: None,
            land: Vec::new(),
            known: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        self.terminal_failure = None;
        self.rows.reset();
        self.arena.reset();
        self.land.clear();
        self.known.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.arena.allocated_bytes()
            + self.land.capacity() * std::mem::size_of::<(u32, u64)>()
            + self.known.capacity() * std::mem::size_of::<(usize, KnownUnit)>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        c.rows = c.rows.max(self.rows.rows.len());
        c.arena = c.arena.max(self.arena.first.len());
        self.rows.grow(&mut c.rows);
        self.arena.grow(&mut c.arena);
    }

    pub fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        census: Option<&UnitInputs>,
        store: Option<&mut ProvinceUnitStore>,
    ) -> CaptureReport {
        CaptureReport {
            ticks: self.capture_with_census(
                memory,
                context.base,
                context.game_state,
                census,
                store,
            ),
        }
    }

    pub fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        CaptureReport::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.capture_with_census(memory, base, state, None, None)
    }

    fn capture_with_census(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        census: Option<&UnitInputs>,
        store: Option<&mut ProvinceUnitStore>,
    ) -> [u64; N] {
        self.reset();
        let _reads = collector_memory::read_accounting::table(TableId::ProvinceUnitOrder);
        let begin = qpc();
        match store {
            Some(store) => {
                let hour = store.begin();
                let Some(header) = cprovince::unit_order_header(memory, state) else {
                    self.rows.outcome = Outcome::Unavailable {
                        reason: "province order root unreadable",
                    };
                    store.measure();
                    return [qpc().saturating_sub(begin).max(0) as u64];
                };
                let slots = hour.carryable
                    && header == store.held.header
                    && cprovince::unit_order_slots(memory, header, &mut store.slots).is_ok();
                let carried = slots
                    && store.slots == store.held.pointers
                    && self.carry(memory, base, state, census, store, hour);
                if !carried {
                    self.walk(memory, base, state, census);
                    if let Outcome::Rows { len } = self.rows.outcome {
                        if let Some(epoch) = hour.epoch
                            && (slots
                                || cprovince::unit_order_slots(memory, header, &mut store.slots)
                                    .is_ok())
                            && store.slots.len() == len
                            && store
                                .held
                                .hold(
                                    header,
                                    &mut store.slots,
                                    &self.rows.rows[..len],
                                    &self.arena.first[..self.arena.len],
                                    &self.land,
                                )
                                .is_ok()
                        {
                            store.held.epoch = Some(epoch);
                        }
                        province_unit_changes::record(
                            &Counts {
                                read: len as u64,
                                full: true,
                                ..Counts::default()
                            },
                            Some(len + 1),
                        );
                    }
                }
                store.measure();
            }
            None => self.walk(memory, base, state, census),
        }
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        census: Option<&UnitInputs>,
    ) {
        self.admit_census(base, census);
        let known = &self.known;
        let land = &mut self.land;
        let acquired = cprovince::walk_unit_order_with_known(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.arena.first,
            |unit| census_unit(known, unit),
            |province, handle| {
                reserve(land, 1).map_err(|_| "province land membership allocation failed")?;
                land.push((province, handle));
                Ok(())
            },
        );
        self.rows.outcome = acquired.outcome;
        self.arena.len = acquired.len;
        self.arena.required = acquired.required;
    }

    fn carry(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        census: Option<&UnitInputs>,
        store: &mut ProvinceUnitStore,
        hour: Hour,
    ) -> bool {
        let provinces = store.held.counts.len() + 1;
        store
            .listed
            .retain(|&id| (1..provinces as u32).contains(&id));
        if !grow_vec(&mut self.rows.rows, provinces - 1) {
            return false;
        }
        store.rows.clear();
        store.handles.clear();
        store.land.clear();
        let fresh = if store.listed.is_empty() {
            Outcome::Rows { len: 0 }
        } else {
            self.admit_census(base, census);
            let known = &self.known;
            let fresh_land = &mut store.land;
            let walk = cprovince::walk_unit_order_listed(
                memory,
                base,
                state,
                &store.listed,
                &mut store.rows,
                &mut store.handles,
                |unit| census_unit(known, unit),
                |province, handle| {
                    reserve(fresh_land, 1)
                        .map_err(|_| "province land membership allocation failed")?;
                    fresh_land.push((province, handle));
                    Ok(())
                },
            );
            store.handles.truncate(walk.len);
            walk.outcome
        };
        if fresh
            != (Outcome::Rows {
                len: store.listed.len(),
            })
        {
            self.rows.outcome = match fresh {
                Outcome::Rows { .. } => Outcome::Unavailable {
                    reason: "province order listed walk truncated",
                },
                failure => failure,
            };
            return true;
        }
        let held = &store.held;
        let replaced: usize = store
            .listed
            .iter()
            .map(|&id| held.counts[id as usize - 1].iter().sum::<u32>() as usize)
            .sum();
        let total = held.handles.len() - replaced + store.handles.len();
        if !grow_vec(&mut self.arena.first, total)
            || advisory(|| reserve(&mut self.land, held.land.len() + store.land.len())).is_err()
        {
            return false;
        }
        let counts = Counts {
            reused: (provinces - 1 - store.listed.len()) as u64,
            read: store.listed.len() as u64,
            quiet: hour.marks == 0,
            ..Counts::default()
        };
        let (mut used, mut from, mut held_land, mut fresh_land, mut next) = (0, 0, 0, 0, 0);
        for id in 1..provinces as u32 {
            let [mixed, naval] = held.counts[id as usize - 1];
            let held_len = (mixed + naval) as usize;
            let held_end = held_land
                + held.land[held_land..]
                    .iter()
                    .take_while(|visit| visit.0 == id)
                    .count();
            let row = if store.listed.get(next) == Some(&id) {
                let row = store.rows[next];
                let fresh_end = fresh_land
                    + store.land[fresh_land..]
                        .iter()
                        .take_while(|visit| visit.0 == id)
                        .count();
                let start = used;
                for range in [row.mixed, row.naval] {
                    self.arena.first[used..used + range[1]]
                        .copy_from_slice(&store.handles[range[0]..][..range[1]]);
                    used += range[1];
                }
                self.land
                    .extend_from_slice(&store.land[fresh_land..fresh_end]);
                fresh_land = fresh_end;
                next += 1;
                UnitOrderRow {
                    province: id,
                    mixed: [start, row.mixed[1]],
                    naval: [start + row.mixed[1], row.naval[1]],
                }
            } else {
                self.arena.first[used..used + held_len]
                    .copy_from_slice(&held.handles[from..from + held_len]);
                self.land.extend_from_slice(&held.land[held_land..held_end]);
                let row = UnitOrderRow {
                    province: id,
                    mixed: [used, mixed as usize],
                    naval: [used + mixed as usize, naval as usize],
                };
                used += held_len;
                row
            };
            self.rows.rows[id as usize - 1] = row;
            from += held_len;
            held_land = held_end;
        }
        self.rows.outcome = Outcome::Rows { len: provinces - 1 };
        self.arena.len = used;
        self.arena.required = used;
        province_unit_changes::record(&counts, Some(provinces));
        for (&id, row) in store.listed.iter().zip(&store.rows) {
            store.held.counts[id as usize - 1] = [row.mixed[1] as u32, row.naval[1] as u32];
        }
        if store
            .held
            .hold_occupants(&self.arena.first[..used], &self.land)
            .is_ok()
        {
            store.held.epoch = hour.epoch;
        }
        true
    }

    fn admit_census(&mut self, base: usize, census: Option<&UnitInputs>) {
        self.known.clear();
        let Some(census) = census else {
            return;
        };
        let seeds = census.seeds();
        let (Some(primary), Some(secondary)) = (
            engine_sites::at(base, carmy::VTABLE_RVA),
            engine_sites::at(base, carmy::SECONDARY_VTABLE_RVA),
        ) else {
            return;
        };
        if !matches!(census.outcome(), Outcome::Rows { len } if len == seeds.len())
            || engine_allocation::advisory(|| reserve(&mut self.known, seeds.len())).is_err()
        {
            return;
        }
        for seed in seeds {
            let (Some(handle), Some(location)) = (seed.handle, seed.location) else {
                continue;
            };
            if seed.pointer != 0
                && seed.primary_vtable == Some(primary as u64)
                && seed.secondary_vtable == Some(secondary as u64)
                && seed.class == Some(0)
                && handle == seed.division
                && handle >> 32 != 0
            {
                self.known
                    .push((seed.pointer, KnownUnit { handle, location }));
            }
        }
        self.known.sort_unstable_by_key(|entry| entry.0);
        let (mut write, mut read) = (0, 0);
        while read < self.known.len() {
            let entry = self.known[read];
            let mut end = read + 1;
            let mut agree = true;
            while end < self.known.len() && self.known[end].0 == entry.0 {
                agree &= self.known[end].1 == entry.1;
                end += 1;
            }
            if agree {
                self.known[write] = entry;
                write += 1;
            }
            read = end;
        }
        self.known.truncate(write);
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self
            .terminal_failure
            .map_or(self.rows.outcome, |reason| Outcome::Unavailable { reason })]
    }
    pub fn land(&self) -> Result<&[(u32, u64)], &'static str> {
        match self.outcomes()[0] {
            Outcome::Rows { .. } => Ok(&self.land),
            Outcome::Unavailable { reason } => Err(reason),
            Outcome::Truncated { .. } => Err("border demand province unit order truncated"),
        }
    }

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::ProvinceUnitOrder, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ProvinceUnitOrderRow {
                province: r.province,
                mixed: owned_vec(copy_vec(
                    &self.arena.first[r.mixed[0]..r.mixed[0] + r.mixed[1]],
                )),
                naval: owned_vec(copy_vec(
                    &self.arena.first[r.naval[0]..r.naval[0] + r.naval[1]],
                )),
            },
        )]
    }

    pub fn table_inputs(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> [replay_frame::frame::TableInput; N] {
        [collector_projection::projection::project_table::<tables::ProvinceUnitOrder, _>(
            projection,
            self.outcomes()[0],
            &self.rows.rows,
            |r| u128::from(r.province),
            |r, prev| {
                r.province == prev.province
                    && prev.mixed.as_slice() == &self.arena.first[r.mixed[0]..r.mixed[0] + r.mixed[1]]
                    && prev.naval.as_slice() == &self.arena.first[r.naval[0]..r.naval[0] + r.naval[1]]
            },
            |r| ProvinceUnitOrderRow {
                province: r.province,
                mixed: owned_vec(copy_vec(
                    &self.arena.first[r.mixed[0]..r.mixed[0] + r.mixed[1]],
                )),
                naval: owned_vec(copy_vec(
                    &self.arena.first[r.naval[0]..r.naval[0] + r.naval[1]],
                )),
            },
        )]
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod owned_tests {
    use super::*;

    #[cfg(test)]
    fn allocations(c: &Captured) -> [(usize, usize, usize); 2] {
        fn vector<T>(v: &Vec<T>) -> (usize, usize, usize) {
            (v.as_ptr() as usize, v.len(), v.capacity())
        }
        [vector(&c.rows.rows), vector(&c.arena.first)]
    }

    #[test]
    fn owned_failure_gates_publication_and_preserves_primary_growth_demand() {
        let initial = Capacities { rows: 2, arena: 2 };
        for (outcome, expected_rows) in [
            (Outcome::Rows { len: 2 }, 4),
            (
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 5,
                },
                8,
            ),
        ] {
            let mut capacities = initial;
            let mut expected_capacity = initial;
            let mut frame = Captured::with_capacities(&initial);
            let mut owned = std::mem::replace(&mut frame, Captured::vacant());
            let mut expected = Captured::with_capacities(&initial);
            assert_eq!(frame.allocated_bytes(), 0);
            for capture in [&mut owned, &mut expected] {
                capture.rows.outcome = outcome;
                capture.rows.rows[0].mixed = [usize::MAX, 1];
                capture.arena.len = 1;
                capture.arena.required = 9;
            }
            let before = allocations(&owned);
            let report = owned.fail_owned_capture("owned acquisition failed");
            assert_eq!(report.ticks, [0]);
            assert_eq!(allocations(&owned), before);
            assert_eq!(owned.rows.outcome, outcome);
            assert_eq!((owned.arena.len, owned.arena.required), (1, 9));
            assert_eq!(
                owned.outcomes(),
                [Outcome::Unavailable {
                    reason: "owned acquisition failed"
                }]
            );
            assert!(matches!(owned.tables()[0], TableView::Unavailable { .. }));
            frame = owned;
            frame.grow(&mut capacities);
            expected.grow(&mut expected_capacity);
            assert!(capacities == expected_capacity);
            assert_eq!((capacities.rows, capacities.arena), (expected_rows, 16));
            assert_eq!(
                allocations(&frame).map(|(_, len, cap)| (len, cap)),
                allocations(&expected).map(|(_, len, cap)| (len, cap))
            );
            let retained = allocations(&frame);
            frame.reset();
            frame.grow(&mut capacities);
            assert!(frame.terminal_failure.is_none());
            assert_eq!((frame.arena.len, frame.arena.required), (0, 0));
            assert_eq!(allocations(&frame), retained);
        }
    }

    #[test]
    fn only_complete_census_divisions_with_their_own_identity_are_known() {
        use engine_native::carmy::DivisionSeed;
        const BASE: usize = 0x1_4000_0000;
        let handle = |id: u64| (id << 32) | 4713;
        let division = |pointer: usize, id: u64| DivisionSeed {
            pointer,
            division: handle(id),
            primary_vtable: Some((BASE + carmy::VTABLE_RVA.0) as u64),
            secondary_vtable: Some((BASE + carmy::SECONDARY_VTABLE_RVA.0) as u64),
            handle: Some(handle(id)),
            class: Some(0),
            location: Some(0x9000),
            ..Default::default()
        };
        let seeds = vec![
            division(0x3000, 1),
            division(0x1000, 2),
            division(0x2000, 3),
            DivisionSeed {
                class: Some(1),
                ..division(0x4000, 4)
            },
            DivisionSeed {
                secondary_vtable: None,
                ..division(0x5000, 5)
            },
            DivisionSeed {
                location: None,
                ..division(0x6000, 6)
            },
            DivisionSeed {
                handle: Some(4713),
                division: 4713,
                ..division(0x7000, 7)
            },
            division(0x8000, 8),
            DivisionSeed {
                location: Some(0x9100),
                ..division(0x8000, 8)
            },
            division(0x1000, 2),
        ];
        let known = |id: u64| KnownUnit {
            handle: handle(id),
            location: 0x9000,
        };
        let mut captured = Captured::with_capacities(&Capacities { rows: 0, arena: 0 });
        let complete = UnitInputs::fixture(seeds.clone(), Outcome::Rows { len: seeds.len() });
        captured.admit_census(BASE, Some(&complete));
        assert_eq!(
            captured.known,
            [(0x1000, known(2)), (0x2000, known(3)), (0x3000, known(1))]
        );
        for incomplete in [
            Outcome::Unavailable {
                reason: "division census not captured",
            },
            Outcome::Truncated {
                len: seeds.len(),
                unread_rows: 1,
            },
        ] {
            captured.admit_census(BASE, Some(&UnitInputs::fixture(seeds.clone(), incomplete)));
            assert!(captured.known.is_empty());
        }
        captured.admit_census(BASE, Some(&complete));
        captured.admit_census(BASE, None);
        assert!(captured.known.is_empty());
    }

    struct World {
        bytes: Vec<u8>,
        reads: std::cell::RefCell<Vec<usize>>,
        fault: Option<usize>,
    }
    const STATE: usize = 0x1000;
    const SLOTS: usize = 0x1800;
    fn province(id: u32) -> usize {
        0x2000 + 0x200 * (id as usize - 1)
    }
    fn lane(id: u32) -> usize {
        0x3000 + 0x100 * (id as usize - 1)
    }
    fn unit(index: u64) -> usize {
        0x6000 + 0x100 * index as usize
    }
    fn handle(index: u64) -> u64 {
        ((index + 1) << 32) | 4713
    }
    impl World {
        fn new() -> Self {
            let mut world = Self {
                bytes: vec![0; 0x7000],
                reads: Default::default(),
                fault: None,
            };
            world.header(STATE + engine_native::cgame_state::PROVINCES, SLOTS, 4);
            for id in 1..=3 {
                world.word(SLOTS + 8 * id as usize, province(id) as u64);
                world.word(province(id), cprovince::VTABLE_RVA.0 as u64);
                world.bytes[province(id) + cprovince::ID..][..4].copy_from_slice(&id.to_le_bytes());
            }
            world.occupy(1, &[0, 1]);
            world.occupy(2, &[2]);
            world
        }
        fn word(&mut self, at: usize, value: u64) {
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn header(&mut self, at: usize, data: usize, count: u32) {
            self.word(at, data as u64);
            self.bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
            self.bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        }
        fn occupy(&mut self, id: u32, units: &[u64]) {
            self.header(
                province(id) + cprovince::OCCUPANTS,
                lane(id),
                units.len() as u32,
            );
            for (at, &index) in units.iter().enumerate() {
                self.word(lane(id) + 8 * at, unit(index) as u64);
            }
        }
        fn census(&self) -> UnitInputs {
            let listed = |id: u32, index: u64| {
                let header = province(id) + cprovince::OCCUPANTS;
                let count = u32::from_le_bytes(self.bytes[header + 12..][..4].try_into().unwrap());
                (0..count as usize).any(|at| {
                    self.bytes[lane(id) + 8 * at..][..8] == (unit(index) as u64).to_le_bytes()
                })
            };
            let seeds: Vec<_> = (0..3)
                .map(|index| engine_native::carmy::DivisionSeed {
                    pointer: unit(index),
                    division: handle(index),
                    primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
                    secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA.0 as u64),
                    handle: Some(handle(index)),
                    class: Some(0),
                    location: Some((1..=3).find(|&id| listed(id, index)).map_or(0, province)),
                    ..Default::default()
                })
                .collect();
            UnitInputs::fixture(seeds, Outcome::Rows { len: 3 })
        }
        #[cfg(test)]
        fn read(&self, id: u32) -> bool {
            self.reads.borrow().contains(&province(id))
        }
    }
    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            let Some(bytes) = self
                .bytes
                .get(address..address + output.len())
                .filter(|_| self.fault != Some(address))
            else {
                return false;
            };
            output.copy_from_slice(bytes);
            true
        }
    }

    fn shared_store() -> (&'static province_unit_changes::Domain, ProvinceUnitStore) {
        let domain = Box::leak(Box::new(province_unit_changes::Domain::enabled(64)));
        (domain, ProvinceUnitStore::new(domain))
    }

    fn hour(
        captured: &mut Captured,
        world: &World,
        store: &mut ProvinceUnitStore,
    ) -> Vec<(u32, Vec<u64>)> {
        world.reads.borrow_mut().clear();
        let census = world.census();
        captured.capture_with_census(world, 0, STATE, Some(&census), Some(store));
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 3 }]);
        let land = captured.land().unwrap().to_vec();
        captured.rows.rows[..3]
            .iter()
            .map(|row| {
                let mixed = captured.arena.first[row.mixed[0]..][..row.mixed[1]].to_vec();
                let visits: Vec<_> = land
                    .iter()
                    .filter(|visit| visit.0 == row.province)
                    .map(|visit| visit.1)
                    .collect();
                assert_eq!(visits, mixed);
                (row.province, mixed)
            })
            .collect()
    }

    #[cfg(test)]
    fn reads(world: &World) -> Vec<bool> {
        (1..=3).map(|id| world.read(id)).collect()
    }

    pub fn held_store() -> Box<ProvinceUnitStore> {
        let (_, mut store) = shared_store();
        let mut captured = Captured::with_capacities(&Capacities::default());
        hour(&mut captured, &World::new(), &mut store);
        assert!(store.holds());
        Box::new(store)
    }

    #[test]
    fn unmarked_provinces_carry_and_marked_ones_read_their_new_order() {
        let mut world = World::new();
        let (domain, mut store) = shared_store();
        let mut frames = [(); 2].map(|_| Captured::with_capacities(&Capacities::default()));
        let mut at = 0;
        let mut next = |world: &World, store: &mut ProvinceUnitStore| {
            at ^= 1;
            frames[at].reset();
            hour(&mut frames[at], world, store)
        };
        let first = next(&world, &mut store);
        assert_eq!(
            first,
            [
                (1, vec![handle(0), handle(1)]),
                (2, vec![handle(2)]),
                (3, vec![])
            ]
        );
        assert_eq!(reads(&world), [true; 3]);
        assert_eq!(next(&world, &mut store), first);
        assert_eq!(reads(&world), [false; 3]);
        world.occupy(1, &[1, 0]);
        drop(domain.notify(1, Some(1)));
        drop(domain.notify(0, Some(1)));
        let moved = next(&world, &mut store);
        assert_eq!(moved[0], (1, vec![handle(1), handle(0)]));
        assert_eq!(moved[1..], first[1..]);
        assert_eq!(reads(&world), [true, false, false]);
        assert_eq!(next(&world, &mut store), moved);
        assert_eq!(reads(&world), [false; 3]);
        domain.boundary(true);
        assert_eq!(next(&world, &mut store), moved);
        assert_eq!(reads(&world), [true; 3]);
    }

    #[test]
    fn a_replaced_province_object_is_read() {
        let mut world = World::new();
        world.occupy(2, &[]);
        world.occupy(3, &[2]);
        let (_, mut store) = shared_store();
        let mut captured = Captured::with_capacities(&Capacities::default());
        assert_eq!(
            hour(&mut captured, &world, &mut store)[2],
            (3, vec![handle(2)])
        );
        hour(&mut captured, &world, &mut store);
        assert_eq!(reads(&world), [false; 3]);
        let replacement = 0x4000;
        world.word(replacement, cprovince::VTABLE_RVA.0 as u64);
        world.bytes[replacement + cprovince::ID..][..4].copy_from_slice(&3_u32.to_le_bytes());
        world.word(SLOTS + 8 * 3, replacement as u64);
        assert_eq!(hour(&mut captured, &world, &mut store)[2], (3, vec![]));
        assert!(world.reads.borrow().contains(&replacement));
    }

    #[test]
    fn a_walk_that_does_not_complete_holds_nothing() {
        for job_failed in [false, true] {
            let mut world = World::new();
            let (domain, mut store) = shared_store();
            let mut frames = [(); 2].map(|_| Captured::with_capacities(&Capacities::default()));
            hour(&mut frames[0], &world, &mut store);
            frames[1].reset();
            hour(&mut frames[1], &world, &mut store);
            assert_eq!(reads(&world), [false; 3]);
            world.occupy(1, &[1, 0]);
            drop(domain.notify(0, Some(1)));
            frames[0].reset();
            if job_failed {
                hour(&mut frames[0], &world, &mut store);
                frames[0].fail_owned_capture("province unit order acquisition panicked");
                store.forget();
            } else {
                world.fault = Some(lane(1));
                let census = world.census();
                frames[0].capture_with_census(&world, 0, STATE, Some(&census), Some(&mut store));
                assert!(matches!(
                    frames[0].outcomes()[0],
                    Outcome::Unavailable { .. }
                ));
                world.fault = None;
            }
            assert!(!store.holds());
            frames[1].reset();
            let after = hour(&mut frames[1], &world, &mut store);
            assert_eq!(reads(&world), [true; 3], "{job_failed}");
            assert_eq!(after[0], (1, vec![handle(1), handle(0)]));
        }
    }

    #[test]
    #[ignore = "release benchmark: ProvinceUnitOrder full walk against the carry"]
    fn province_unit_carry_scale_bench() {
        if cfg!(debug_assertions) {
            panic!("run the carry benchmark in release");
        }
        const PROVINCES: usize = 13_000;
        const RECORD: usize = 0x400;
        let units_per = |id: usize| {
            if id.is_multiple_of(6) {
                1 + (id / 6) % 3
            } else {
                0
            }
        };
        let units: usize = (1..=PROVINCES).map(units_per).sum();
        let records = 0x10_0000;
        let unit_area = records + (PROVINCES + 1) * RECORD;
        let mut heap = vec![0_u8; unit_area + units * 0x100];
        let base = heap.as_ptr() as usize;
        let put = |heap: &mut Vec<u8>, at: usize, value: &[u8]| {
            heap[at..at + value.len()].copy_from_slice(value)
        };
        let state = 0x100;
        let slots = 0x1000;
        let root = state + engine_native::cgame_state::PROVINCES;
        put(&mut heap, root, &((base + slots) as u64).to_le_bytes());
        put(&mut heap, root + 8, &((PROVINCES + 1) as u32).to_le_bytes());
        put(
            &mut heap,
            root + 12,
            &((PROVINCES + 1) as u32).to_le_bytes(),
        );
        let mut seeds = Vec::new();
        for id in 1..=PROVINCES {
            let record = records + id * RECORD;
            put(
                &mut heap,
                slots + 8 * id,
                &((base + record) as u64).to_le_bytes(),
            );
            put(
                &mut heap,
                record,
                &((base + cprovince::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut heap,
                record + cprovince::ID,
                &(id as u32).to_le_bytes(),
            );
            let count = units_per(id);
            let lane = record + 0x200;
            let header = record + cprovince::OCCUPANTS;
            put(&mut heap, header, &((base + lane) as u64).to_le_bytes());
            put(&mut heap, header + 8, &(count as u32).to_le_bytes());
            put(&mut heap, header + 12, &(count as u32).to_le_bytes());
            for at in 0..count {
                let unit = base + unit_area + seeds.len() * 0x100;
                put(&mut heap, lane + 8 * at, &(unit as u64).to_le_bytes());
                let handle = ((seeds.len() as u64 + 1) << 32) | 4713;
                seeds.push(engine_native::carmy::DivisionSeed {
                    pointer: unit,
                    division: handle,
                    primary_vtable: Some((base + carmy::VTABLE_RVA.0) as u64),
                    secondary_vtable: Some((base + carmy::SECONDARY_VTABLE_RVA.0) as u64),
                    handle: Some(handle),
                    class: Some(0),
                    location: Some(base + record),
                    ..Default::default()
                });
            }
        }
        let census = UnitInputs::fixture(seeds.clone(), Outcome::Rows { len: seeds.len() });
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let domain = Box::leak(Box::new(province_unit_changes::Domain::enabled(
            PROVINCES + 1,
        )));
        let mut store = ProvinceUnitStore::new(domain);
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut time = |store: Option<&mut ProvinceUnitStore>, marked: usize| {
            for id in (1..=PROVINCES)
                .step_by(PROVINCES / marked.max(1))
                .take(marked)
            {
                drop(domain.notify(0, Some(id)));
            }
            captured.reset();
            let begin = std::time::Instant::now();
            captured.capture_with_census(&memory, base, base + state, Some(&census), store);
            let elapsed = begin.elapsed().as_secs_f64() * 1e3;
            assert_eq!(captured.outcomes(), [Outcome::Rows { len: PROVINCES }]);
            elapsed
        };
        let median = |mut samples: Vec<f64>| {
            samples.sort_by(f64::total_cmp);
            samples[samples.len() / 2]
        };
        let full = median((0..41).map(|_| time(None, 0)).collect());
        time(Some(&mut store), 0);
        let quiet = median((0..41).map(|_| time(Some(&mut store), 0)).collect());
        let marked = median((0..41).map(|_| time(Some(&mut store), 150)).collect());
        assert!(store.holds());
        println!(
            "PROVINCE_UNIT_CARRY provinces={PROVINCES} divisions={units} full_ms={full:.3} quiet_ms={quiet:.3} marked150_ms={marked:.3}"
        );
        drop(heap);
    }

    #[test]
    fn owned_and_direct_capture_keep_one_first_fault_read_after_previous_failure() {
        use std::cell::RefCell;
        #[derive(Default)]
        struct Unreadable(RefCell<Vec<(usize, usize)>>);
        impl Memory for Unreadable {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.borrow_mut().push((address, output.len()));
                false
            }
        }
        let capacities = Capacities { rows: 2, arena: 3 };
        let context = CaptureContext {
            base: 0x140000000,
            game_state: 0x1000,
        };
        let mut direct = Captured::with_capacities(&capacities);
        let mut owned = Captured::with_capacities(&capacities);
        let before = allocations(&owned);
        let direct_memory = Unreadable::default();
        let owned_memory = Unreadable::default();
        direct.capture(&direct_memory, context.base, context.game_state);
        for owned_entry in [false, true] {
            owned.fail_owned_capture("previous request failed");
            owned_memory.0.borrow_mut().clear();
            if owned_entry {
                owned.capture_owned(&owned_memory, context, None, None);
            } else {
                owned.capture(&owned_memory, context.base, context.game_state);
            }
            assert!(owned.terminal_failure.is_none());
            assert_eq!(*owned_memory.0.borrow(), *direct_memory.0.borrow());
            assert_eq!(owned_memory.0.borrow().len(), 1);
            assert_eq!(owned.outcomes(), direct.outcomes());
            assert_eq!(owned.tables(), direct.tables());
            assert_eq!((owned.arena.len, owned.arena.required), (0, 0));
            assert_eq!(allocations(&owned), before);
        }
    }
}
