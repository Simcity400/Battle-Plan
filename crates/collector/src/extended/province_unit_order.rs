//! Hourly ordered native inputs with independently bounded owned payloads.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
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
    pub needed: [bool; N],
}

/// Results in TABLES order. Terminal worker failure has no completed timing
/// sample, so its zero tick is explicitly unmeasured rather than free work.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub matched: [bool; N],
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
}
impl Captured {
    /// Allocation-free placeholder while the original buffers are owned by an
    /// acquisition request. Restore that request before publication or growth.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(
            &Capacities { rows: 0, arena: 0 },
            &[crate::sampling::CONSISTENCY_TARGET; N],
        )
    }

    pub fn with_capacities(c: &Capacities, s: &[u16]) -> Self {
        assert_eq!(s.len(), N);
        Self {
            rows: CaptureRows::new(c.rows, s[0]),
            arena: Arena::new(c.arena, s[0]),
            terminal_failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.terminal_failure = None;
        self.rows.reset();
        self.arena.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.arena.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities, s: &[u16]) {
        self.rows.grow(&mut c.rows, s[0]);
        self.arena.grow(&mut c.arena, s[0]);
    }

    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        let (matched, ticks) =
            self.capture(memory, context.base, context.game_state, &context.needed);
        CaptureReport { matched, ticks }
    }

    /// Convert an accepted panic/stop into a terminal publication failure.
    /// Preserve the original walk's outcome and arena pressure so off-thread
    /// growth sees the same observed demand; partial rows remain inaccessible.
    pub(crate) fn fail_owned_capture(
        &mut self,
        context: CaptureContext,
        reason: &'static str,
    ) -> CaptureReport {
        self.terminal_failure = Some(reason);
        self.rows.checked = context.needed[0];
        self.rows.mismatches = usize::from(context.needed[0]);
        CaptureReport::default()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        // Parent capture already resets frames in production. Repeat the
        // metadata reset here so a direct retry after a terminal owned result
        // cannot retain its publication gate or prior growth metadata.
        self.reset();
        let begin = qpc();
        let first = cprovince::walk_unit_order(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.arena.first,
        );
        self.rows.outcome = first.outcome;
        self.rows.checked = needed[0];
        self.arena.len = first.len;
        self.arena.required = first.required;
        let matched = if needed[0] {
            let second = cprovince::walk_unit_order(
                memory,
                base,
                state,
                &mut self.rows.second,
                &mut self.arena.second,
            );
            let rows = self.rows.compare(second.outcome);
            let arena = self.arena.compare(second.len, second.required);
            if !arena {
                self.rows.mismatches += 1;
            }
            rows && arena
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        if let Some(reason) = self.terminal_failure {
            return [Outcome::Unavailable { reason }; N];
        }
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "province_unit_order changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::ProvinceUnitOrder, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ProvinceUnitOrderRow {
                province: r.province,
                mixed: self.arena.first[r.mixed[0]..r.mixed[0] + r.mixed[1]].to_vec(),
                naval: self.arena.first[r.naval[0]..r.naval[0] + r.naval[1]].to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod owned_tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;

    fn allocations(c: &Captured) -> [(usize, usize, usize); 4] {
        fn vector<T>(v: &Vec<T>) -> (usize, usize, usize) {
            (v.as_ptr() as usize, v.len(), v.capacity())
        }
        [
            vector(&c.rows.rows),
            vector(&c.rows.second),
            vector(&c.arena.first),
            vector(&c.arena.second),
        ]
    }

    #[test]
    fn owned_failure_gates_publication_and_preserves_growth_demand() {
        let initial = Capacities { rows: 2, arena: 2 };
        for needed in [false, true] {
            let successful = if needed { 0 } else { CONSISTENCY_TARGET };
            let mut capacities = initial;
            let mut expected_capacity = initial;
            let mut frame = Captured::with_capacities(&initial, &[successful]);
            let mut owned = std::mem::replace(&mut frame, Captured::vacant());
            let mut expected = Captured::with_capacities(&initial, &[successful]);
            assert_eq!(frame.allocated_bytes(), 0);

            for capture in [&mut owned, &mut expected] {
                capture.rows.outcome = Outcome::Truncated {
                    len: 1,
                    unread_rows: 5,
                };
                capture.rows.rows[0].mixed = [usize::MAX, 1];
                capture.arena.len = 1;
                capture.arena.required = 3;
                assert!(!capture.arena.compare(1, 9));
            }
            let before = allocations(&owned);
            let report = owned.fail_owned_capture(
                CaptureContext {
                    base: 0,
                    game_state: 0,
                    needed: [needed],
                },
                "owned province unit order failed",
            );
            assert_eq!(report.matched, [false]);
            assert_eq!(report.ticks, [0]);
            assert_eq!(allocations(&owned), before);
            assert_eq!(owned.rows.checked, needed);
            assert_eq!(owned.checked_mismatches(), [usize::from(needed)]);
            assert_eq!(
                owned.rows.outcome,
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 5
                }
            );
            assert_eq!((owned.arena.len, owned.arena.required), (1, 9));
            assert_eq!(
                owned.outcomes(),
                [Outcome::Unavailable {
                    reason: "owned province unit order failed"
                }]
            );
            assert!(matches!(owned.tables()[0], TableView::Unavailable { .. }));

            frame = owned;
            frame.grow(&mut capacities, &[successful]);
            expected.grow(&mut expected_capacity, &[successful]);
            assert!(capacities == expected_capacity);
            assert_eq!((capacities.rows, capacities.arena), (8, 16));
            assert_eq!(
                allocations(&frame).map(|(_, len, capacity)| (len, capacity)),
                allocations(&expected).map(|(_, len, capacity)| (len, capacity))
            );
            assert_eq!(!frame.rows.second.is_empty(), needed);
            assert_eq!(!frame.arena.second.is_empty(), needed);

            frame.reset();
            assert!(frame.terminal_failure.is_none());
            assert_eq!(frame.checked_mismatches(), [0]);
            assert!(!frame.rows.checked);
            assert_eq!((frame.arena.len, frame.arena.required), (0, 0));
        }
    }

    #[test]
    fn owned_and_direct_capture_clear_terminal_failure_and_keep_read_order() {
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
        for needed in [false, true] {
            let successful = if needed { 0 } else { CONSISTENCY_TARGET };
            let context = CaptureContext {
                base: 0x140000000,
                game_state: 0x1000,
                needed: [needed],
            };
            let mut direct = Captured::with_capacities(&capacities, &[successful]);
            let mut owned = Captured::with_capacities(&capacities, &[successful]);
            let direct_memory = Unreadable::default();
            let owned_memory = Unreadable::default();
            let expected = direct
                .capture(
                    &direct_memory,
                    context.base,
                    context.game_state,
                    &context.needed,
                )
                .0;

            for owned_entry in [false, true] {
                owned.fail_owned_capture(
                    CaptureContext {
                        needed: [!needed],
                        ..context
                    },
                    "previous request failed",
                );
                owned_memory.0.borrow_mut().clear();
                let actual = if owned_entry {
                    owned.capture_owned(&owned_memory, context).matched
                } else {
                    owned
                        .capture(
                            &owned_memory,
                            context.base,
                            context.game_state,
                            &context.needed,
                        )
                        .0
                };
                assert!(owned.terminal_failure.is_none());
                assert_eq!(*owned_memory.0.borrow(), *direct_memory.0.borrow());
                assert_eq!(owned_memory.0.borrow().len(), 1 + usize::from(needed));
                assert_eq!(actual, expected);
                assert_eq!(owned.outcomes(), direct.outcomes());
                assert_eq!(owned.checked_mismatches(), direct.checked_mismatches());
                assert_eq!((owned.arena.len, owned.arena.required), (0, 0));
            }
        }
    }
}
