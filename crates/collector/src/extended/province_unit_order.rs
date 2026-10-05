//! Hourly ordered native inputs with independently bounded owned payloads.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec, reserve};
use engine::{Memory, cprovince, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ProvinceUnitOrderRow, TableId, tables},
};
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceUnitOrder];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
}

/// Acquisition ticks in TABLES order. Interrupted work has no completed timing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    arena: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 16384,
            arena: 32768,
        }
    }
}
pub struct Captured {
    rows: CaptureRows<cprovince::UnitOrderRow>,
    arena: Arena<u64>,
    terminal_failure: Option<&'static str>,
    land: Vec<(u32, u64)>,
}
impl Captured {
    #[cfg(test)]
    pub(super) fn install_land_fixture(&mut self, land: Vec<(u32, u64)>, outcome: Outcome) {
        self.land = land;
        self.rows.outcome = outcome;
    }

    /// Allocation-free placeholder while the original buffers are owned by an
    /// acquisition request. Restore that request before publication or growth.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, arena: 0 })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            arena: Arena::new(c.arena),
            terminal_failure: None,
            land: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        self.terminal_failure = None;
        self.rows.reset();
        self.arena.reset();
        self.land.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.arena.allocated_bytes()
            + self.land.capacity() * std::mem::size_of::<(u32, u64)>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.arena.grow(&mut c.arena);
    }

    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        CaptureReport {
            ticks: self.capture(memory, context.base, context.game_state),
        }
    }

    /// Convert an accepted panic/stop into a terminal publication failure.
    /// Preserve the original walk's outcome and arena pressure so off-thread
    /// growth sees the same observed demand; partial rows remain inaccessible.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        CaptureReport::default()
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.reset();
        let _reads = crate::read_accounting::table(TableId::ProvinceUnitOrder);
        let begin = qpc();
        let land = &mut self.land;
        let acquired = cprovince::walk_unit_order_with_land(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.arena.first,
            |province, handle| {
                reserve(land, 1).map_err(|_| "province land membership allocation failed")?;
                land.push((province, handle));
                Ok(())
            },
        );
        self.rows.outcome = acquired.outcome;
        self.arena.len = acquired.len;
        self.arena.required = acquired.required;
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self
            .terminal_failure
            .map_or(self.rows.outcome, |reason| Outcome::Unavailable { reason })]
    }
    pub(crate) fn land(&self) -> Result<&[(u32, u64)], &'static str> {
        match self.outcomes()[0] {
            Outcome::Rows { .. } => Ok(&self.land),
            Outcome::Unavailable { reason } => Err(reason),
            Outcome::Truncated { .. } => Err("border demand province unit order truncated"),
        }
    }

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
}

#[cfg(test)]
mod owned_tests {
    use super::*;

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
                owned.capture_owned(&owned_memory, context);
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
