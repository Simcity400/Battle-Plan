use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use collector_observers::province_combat_changes::{self, Changes, Counts};
use engine_allocation::{copy_vec, owned_vec, resize_default};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cprovince;
use engine_naval_history::naval_combat_transfers as transfers;
use replay_data::{
    rows::NavalCombatTransferRow, rows::NavalProvinceCombatRow, rows::NavalTransferUnitRow,
    rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

const _: () = assert!(
    cprovince::COMBAT_LAND == NavalProvinceCombatRow::LAND
        && cprovince::COMBAT_BORDER_WAR == NavalProvinceCombatRow::BORDER_WAR
        && cprovince::COMBAT_NAVAL == NavalProvinceCombatRow::NAVAL
);

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::NavalProvinceCombats,
    TableId::NavalCombatTransfers,
    TableId::NavalTransferUnits,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    provinces: usize,
    province_combats: usize,
    combats: usize,
    transfer_handles: usize,
    transfers: usize,
    unit_handles: usize,
}

collector_primitives::capacity_max!(Capacities {
    provinces,
    province_combats,
    combats,
    transfer_handles,
    transfers,
    unit_handles
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut capture = Captured::vacant();
        capture.capture_with(memory, base, state, None);
        let mut failure = None;
        for (floor, outcome) in [
            (&mut self.provinces, capture.provinces.outcome),
            (&mut self.combats, capture.combats.outcome),
            (&mut self.transfers, capture.transfers.outcome),
        ] {
            match collector_extended_support::demand::rows(outcome) {
                Ok(count) => *floor = (*floor).max(count),
                Err(reason) => {
                    failure.get_or_insert(reason);
                }
            }
        }
        self.province_combats = self.province_combats.max(capture.province_combats.required);
        self.transfer_handles = self.transfer_handles.max(capture.transfer_handles.required);
        self.transfers = self.transfers.max(capture.transfers.rows.len());
        self.unit_handles = self.unit_handles.max(capture.unit_handles.required);
        failure.map_or(Ok(()), Err)
    }
}

struct Held {
    epoch: u64,
    header: [u8; 16],
    provinces: Vec<u32>,
}

#[derive(Default)]
pub struct Shelf {
    held: Option<Held>,
    bytes: usize,
}

pub type Home = engine_allocation::SharedBox<std::sync::Mutex<Shelf>>;

pub fn new_home() -> std::io::Result<Home> {
    engine_allocation::shared(std::sync::Mutex::default())
}

fn shelf(home: &Home) -> std::sync::MutexGuard<'_, Shelf> {
    home.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn home_allocated_bytes(home: &Home) -> usize {
    shelf(home).bytes
}

pub struct Captured {
    home: Option<Home>,
    provinces: CaptureRows<cprovince::DirectCombatsRow>,
    listed: Vec<u32>,
    province_combats: Arena<u64>,
    province_kinds: Arena<u8>,
    combats: CaptureRows<transfers::CombatTransfers>,
    transfer_handles: Arena<u64>,
    transfers: CaptureRows<transfers::TransferUnits>,
    unit_handles: Arena<u64>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::without_home(&Capacities::default())
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            home: new_home().ok(),
            ..Self::without_home(c)
        }
    }

    pub fn without_home(c: &Capacities) -> Self {
        Self {
            home: None,
            provinces: CaptureRows::new(c.provinces),
            listed: Vec::new(),
            province_combats: Arena::new(c.province_combats),
            province_kinds: Arena::new(c.province_combats),
            combats: CaptureRows::new(c.combats),
            transfer_handles: Arena::new(c.transfer_handles),
            transfers: CaptureRows::new(c.transfers),
            unit_handles: Arena::new(c.unit_handles),
        }
    }

    pub fn reset(&mut self) {
        self.provinces.reset();
        self.province_combats.reset();
        self.province_kinds.reset();
        self.combats.reset();
        self.transfer_handles.reset();
        self.transfers.reset();
        self.unit_handles.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.provinces.allocated_bytes()
            + self.listed.capacity() * size_of::<u32>()
            + self.province_combats.allocated_bytes()
            + self.province_kinds.allocated_bytes()
            + self.combats.allocated_bytes()
            + self.transfer_handles.allocated_bytes()
            + self.transfers.allocated_bytes()
            + self.unit_handles.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.provinces.grow(&mut c.provinces);
        self.province_combats.grow(&mut c.province_combats);
        self.province_kinds.required = self.province_combats.required;
        if self.province_kinds.first.len() < c.province_combats
            && resize_default(&mut self.province_kinds.first, c.province_combats).is_err()
        {
            return;
        }
        self.combats.grow(&mut c.combats);
        self.transfer_handles.grow(&mut c.transfer_handles);
        self.transfers.grow(&mut c.transfers);
        self.unit_handles.grow(&mut c.unit_handles);
    }

    pub fn home(&self) -> Option<Home> {
        self.home.clone()
    }

    pub fn share_home(&mut self, home: &Home) {
        self.home = Some(home.clone());
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.capture_with(memory, base, state, Some(&province_combat_changes::Observer))
    }

    pub fn capture_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        changes: Option<&dyn Changes>,
    ) -> [u64; N] {
        let begin = qpc();
        let direct = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            self.walk_provinces(memory, base, state, changes)
        };
        self.provinces.outcome = direct.outcome;
        self.province_combats.required = direct.required;
        self.province_combats.len = direct.required.min(self.province_combats.first.len());
        self.province_kinds.required = direct.required;
        self.province_kinds.len = direct.required.min(self.province_kinds.first.len());
        let middle = qpc();
        let walked = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            transfers::walk(
                memory,
                base,
                state,
                transfers::Buffers {
                    combats: &mut self.combats.rows,
                    transfer_handles: &mut self.transfer_handles.first,
                    transfers: &mut self.transfers.rows,
                    unit_handles: &mut self.unit_handles.first,
                },
            )
        };
        self.combats.outcome = walked.combats;
        self.transfers.outcome = walked.transfers;
        self.transfer_handles.required = walked.transfer_handles_required;
        self.unit_handles.required = walked.unit_handles_required;
        self.transfer_handles.len = walked
            .transfer_handles_required
            .min(self.transfer_handles.first.len());
        self.unit_handles.len = walked
            .unit_handles_required
            .min(self.unit_handles.first.len());
        let end = qpc();
        [
            middle.saturating_sub(begin).max(0) as u64,
            end.saturating_sub(middle).max(0) as u64,
            0,
        ]
    }

    fn walk_provinces(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        changes: Option<&dyn Changes>,
    ) -> cprovince::DirectCombatsWalk {
        let home = self
            .home
            .clone()
            .filter(|_| province_combat_changes::CARRY);
        let (Some(home), Some(changes)) = (home, changes) else {
            return cprovince::walk_direct_combats(
                memory,
                base,
                state,
                &mut self.provinces.rows,
                &mut self.province_combats.first,
                &mut self.province_kinds.first,
                None,
            );
        };
        let held = shelf(&home).held.take();
        self.listed.clear();
        let mut complete = true;
        let listed = &mut self.listed;
        let epoch = changes.take(&mut |id| {
            complete &= u32::try_from(id).is_ok_and(|id| {
                engine_allocation::advisory(|| engine_allocation::reserve(listed, 1)).is_ok() && {
                    listed.push(id);
                    true
                }
            });
        });
        let marked = self.listed.len();
        let header = state
            .checked_add(engine_native::cgame_state::PROVINCES)
            .and_then(|root| engine_native_memory::read::<[u8; 16]>(memory, root));
        let slots = header
            .and_then(|header| engine_native_memory::cpdx_array::Array::from_header(header, 8))
            .map(|array| array.count as usize);
        let carried = match (&held, epoch, header) {
            (Some(held), Some(epoch), Some(header))
                if complete && held.epoch == epoch && held.header == header =>
            {
                engine_allocation::advisory(|| {
                    engine_allocation::reserve(&mut self.listed, held.provinces.len())
                })
                .is_ok() && {
                    self.listed.extend_from_slice(&held.provinces);
                    self.listed.sort_unstable();
                    self.listed.dedup();
                    true
                }
            }
            _ => false,
        };
        let walk = cprovince::walk_direct_combats(
            memory,
            base,
            state,
            &mut self.provinces.rows,
            &mut self.province_combats.first,
            &mut self.province_kinds.first,
            carried.then_some(&self.listed[..]),
        );
        let read = if carried {
            self.listed.len()
        } else {
            slots.map_or(0, |slots| slots.saturating_sub(1))
        };
        province_combat_changes::record(
            &Counts {
                reused: slots.map_or(0, |slots| slots.saturating_sub(1)).saturating_sub(read) as u64,
                read: read as u64,
                quiet: carried && marked == 0,
                full: !carried,
            },
            (!carried).then_some(slots).flatten(),
        );
        if let (Outcome::Rows { len }, Some(epoch), Some(header)) = (walk.outcome, epoch, header) {
            let mut provinces = held.map(|held| held.provinces).unwrap_or_default();
            provinces.clear();
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut provinces, len))
                .is_ok()
            {
                provinces.extend(self.provinces.rows[..len].iter().map(|row| row.province));
                let mut shelf = shelf(&home);
                shelf.bytes = shelf.bytes.max(provinces.capacity() * size_of::<u32>());
                shelf.held = Some(Held {
                    epoch,
                    header,
                    provinces,
                });
            }
        }
        walk
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.provinces.outcome,
            self.combats.outcome,
            self.transfers.outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let [provinces, combats, transfers] = self.outcomes();
        fn slice<T: Copy>(arena: &Arena<T>, [start, len]: [usize; 2]) -> Option<Vec<T>> {
            arena
                .first
                .get(start..start.checked_add(len)?)
                .map(|source| owned_vec(copy_vec(source)))
        }
        fn fits<T>(arena: &Arena<T>, [start, len]: [usize; 2]) -> bool {
            start
                .checked_add(len)
                .is_some_and(|end| end <= arena.first.len())
        }
        let written = |outcome: Outcome, len: usize| outcome.len().min(len);
        let valid = self.provinces.rows[..written(provinces, self.provinces.rows.len())]
            .iter()
            .all(|row| {
                fits(&self.province_combats, row.combats) && fits(&self.province_kinds, row.combats)
            })
            && self.combats.rows[..written(combats, self.combats.rows.len())]
                .iter()
                .all(|row| fits(&self.transfer_handles, row.transfers))
            && self.transfers.rows[..written(transfers, self.transfers.rows.len())]
                .iter()
                .all(|row| fits(&self.unit_handles, row.units));
        if !valid {
            return std::array::from_fn(|_| {
                unavailable("naval combat source range outside its arena")
            });
        }
        [
            table::<tables::NavalProvinceCombats, _>(provinces, &self.provinces.rows, |r| {
                NavalProvinceCombatRow {
                    province: r.province,
                    combats: slice(&self.province_combats, r.combats).unwrap_or_default(),
                    kinds: slice(&self.province_kinds, r.combats).unwrap_or_default(),
                }
            }),
            table::<tables::NavalCombatTransfers, _>(combats, &self.combats.rows, |r| {
                NavalCombatTransferRow {
                    combat: r.combat,
                    transfers: slice(&self.transfer_handles, r.transfers).unwrap_or_default(),
                }
            }),
            table::<tables::NavalTransferUnits, _>(transfers, &self.transfers.rows, |r| {
                NavalTransferUnitRow {
                    transfer: r.transfer,
                    units: slice(&self.unit_handles, r.units).unwrap_or_default(),
                }
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    const STATE: usize = 0x1000;
    const SLOTS: usize = 0x1800;
    const PROVINCES: [usize; 3] = [0x2000, 0x2400, 0x2800];
    const VECTORS: [usize; 3] = [0x3000, 0x3100, 0x3200];
    const NAVAL: usize = 0x4000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn world(with: &[usize]) -> Vec<u8> {
        let mut bytes = vec![0; 0x5000];
        let array = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
            put(bytes, at, &(data as u64).to_le_bytes());
            put(bytes, at + 8, &count.to_le_bytes());
            put(bytes, at + 12, &count.to_le_bytes());
        };
        array(&mut bytes, STATE + engine_native::cgame_state::PROVINCES, SLOTS, 4);
        for (index, province) in PROVINCES.into_iter().enumerate() {
            put(&mut bytes, SLOTS + 8 + index * 8, &(province as u64).to_le_bytes());
            put(&mut bytes, province, &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, province + cprovince::ID, &(index as u32 + 1).to_le_bytes());
            let count = u32::from(with.contains(&(index + 1)));
            array(&mut bytes, province + cprovince::COMBATS, VECTORS[index], count);
            put(&mut bytes, VECTORS[index], &(NAVAL as u64).to_le_bytes());
        }
        put(
            &mut bytes,
            NAVAL,
            &(engine_native::cnaval_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            NAVAL + engine_native::cnaval_combat::HANDLE,
            &(7_u64 << 32 | 4713).to_le_bytes(),
        );
        bytes
    }

    fn hour(capture: &mut Captured, bytes: &[u8], changes: &dyn Changes) -> (Outcome, Vec<u32>) {
        let memory = engine_native_memory::memory::SliceMemory { base: 0, bytes };
        capture.reset();
        capture.capture_with(&memory, 0, STATE, Some(changes));
        let outcome = capture.provinces.outcome;
        let len = outcome.len();
        (
            outcome,
            capture.provinces.rows[..len].iter().map(|row| row.province).collect(),
        )
    }

    #[test]
    fn provinces_without_combats_are_read_again_only_when_a_writer_marks_them() {
        let domain = province_combat_changes::Domain::enabled(64);
        let mut capture = Captured::with_capacities(&Capacities::default());
        let held = world(&[2]);
        assert_eq!(hour(&mut capture, &held, &domain), (Outcome::Rows { len: 1 }, vec![2]));
        let mut unread = held.clone();
        put(&mut unread, SLOTS + 8, &0_u64.to_le_bytes());
        assert_eq!(hour(&mut capture, &unread, &domain), (Outcome::Rows { len: 1 }, vec![2]));
        drop(domain.notify(0, Some(3)));
        let both = world(&[2, 3]);
        assert_eq!(hour(&mut capture, &both, &domain), (Outcome::Rows { len: 2 }, vec![2, 3]));
        drop(domain.notify(1, Some(2)));
        let third = world(&[3]);
        assert_eq!(hour(&mut capture, &third, &domain), (Outcome::Rows { len: 1 }, vec![3]));
        domain.boundary(true);
        let mut unread = third.clone();
        put(&mut unread, SLOTS + 8, &0_u64.to_le_bytes());
        assert!(matches!(
            hour(&mut capture, &unread, &domain).0,
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            hour(&mut capture, &unread, &domain).0,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(hour(&mut capture, &third, &domain), (Outcome::Rows { len: 1 }, vec![3]));
    }

    #[test]
    fn cold_source_storage_waits_for_original_headers() {
        let capacity = Capacities::default();
        assert_eq!(
            [
                capacity.provinces,
                capacity.province_combats,
                capacity.combats,
                capacity.transfer_handles,
                capacity.transfers,
                capacity.unit_handles,
            ],
            [0; 6]
        );
        assert_eq!(Captured::with_capacities(&capacity).allocated_bytes(), 0);
        assert_eq!(Captured::vacant().allocated_bytes(), 0);
    }
    #[test]
    fn rows_take_their_arena_ranges_and_a_range_past_the_arena_refuses() {
        let mut capture = Captured::with_capacities(&Capacities {
            provinces: 1,
            province_combats: 2,
            combats: 1,
            transfer_handles: 1,
            transfers: 1,
            unit_handles: 2,
        });
        capture.provinces.rows[0] = cprovince::DirectCombatsRow {
            province: 9,
            combats: [0, 2],
        };
        capture.provinces.outcome = Outcome::Rows { len: 1 };
        capture.province_combats.first.copy_from_slice(&[5, 6]);
        capture
            .province_kinds
            .first
            .copy_from_slice(&[cprovince::COMBAT_NAVAL, cprovince::COMBAT_LAND]);
        capture.combats.rows[0] = transfers::CombatTransfers {
            combat: 5,
            transfers: [0, 1],
        };
        capture.combats.outcome = Outcome::Rows { len: 1 };
        capture.transfer_handles.first[0] = 7;
        capture.transfers.rows[0] = transfers::TransferUnits {
            transfer: 7,
            units: [0, 2],
        };
        capture.transfers.outcome = Outcome::Rows { len: 1 };
        capture.unit_handles.first.copy_from_slice(&[8, 9]);
        let [
            TableView::Complete(TableRows::NavalProvinceCombats(provinces)),
            TableView::Complete(TableRows::NavalCombatTransfers(combats)),
            TableView::Complete(TableRows::NavalTransferUnits(units)),
        ] = capture.tables()
        else {
            panic!("complete tables");
        };
        assert_eq!(provinces[0].combats, [5, 6]);
        assert_eq!(
            provinces[0].kinds,
            [NavalProvinceCombatRow::NAVAL, NavalProvinceCombatRow::LAND]
        );
        assert_eq!(combats[0].transfers, [7]);
        assert_eq!(units[0].units, [8, 9]);
        capture.transfers.rows[0].units = [1, 2];
        assert!(
            capture
                .tables()
                .iter()
                .all(|view| matches!(view, TableView::Unavailable { .. }))
        );
    }
}
