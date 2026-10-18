//! Hourly ordered vectors the naval combat map icons are placed from: each province's combat
//! vector with every entry's class, each naval combat's transfers and each live transfer's live
//! units.
use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec, resize_default};
use engine_native::{Memory, cprovince, walk::Outcome};
use engine_naval_history::naval_combat_transfers as transfers;
use replay_data::{
    rows::NavalCombatTransferRow, rows::NavalProvinceCombatRow, rows::NavalTransferUnitRow,
    rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

// The collector copies the engine's entry classes into the row unchanged.
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
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("naval combat transfers and their units", 1, &[1, 2])];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    provinces: usize,
    province_combats: usize,
    combats: usize,
    transfer_handles: usize,
    transfers: usize,
    unit_handles: usize,
}

impl Capacities {
    /// Live attach/cadence floor for the original map-icon vectors and
    /// transfer/unit rosters. The temporary owned walk keeps distinct-reference
    /// scratch complete while measuring its demand.
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut capture = Captured::vacant();
        capture.capture(memory, base, state);
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
        // Distinct transfer rows also hold every nonzero original occurrence
        // until compaction; the emitted unique row count alone is insufficient.
        self.transfers = self.transfers.max(capture.transfers.rows.len());
        self.unit_handles = self.unit_handles.max(capture.unit_handles.required);
        failure.map_or(Ok(()), Err)
    }
}

pub struct Captured {
    provinces: CaptureRows<cprovince::DirectCombatsRow>,
    province_combats: Arena<u64>,
    /// Each entry's class, at the same index as its handle in `province_combats`.
    province_kinds: Arena<u8>,
    combats: CaptureRows<transfers::CombatTransfers>,
    transfer_handles: Arena<u64>,
    transfers: CaptureRows<transfers::TransferUnits>,
    unit_handles: Arena<u64>,
}

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            provinces: CaptureRows::new(c.provinces),
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

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let begin = qpc();
        let direct = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            cprovince::walk_direct_combats_growing(
                memory,
                base,
                state,
                &mut self.provinces.rows,
                &mut self.province_combats.first,
                &mut self.province_kinds.first,
            )
        };
        self.provinces.outcome = direct.outcome;
        self.province_combats.required = direct.required;
        self.province_combats.len = direct.required.min(self.province_combats.first.len());
        self.province_kinds.required = direct.required;
        self.province_kinds.len = direct.required.min(self.province_kinds.first.len());
        let middle = qpc();
        let walked = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            transfers::walk_growing(
                memory,
                base,
                state,
                transfers::GrowingBuffers {
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
