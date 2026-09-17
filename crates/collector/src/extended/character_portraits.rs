//! Hourly ordered native inputs with independently bounded owned payloads.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccharacter, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CharacterPortraitRow, TableId, tables},
};
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CharacterPortraits];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Inputs for one exclusively owned hourly acquisition. Native readers remain
/// local to the executing thread; needed requests the independent cold pass.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; N],
}

/// Results in TABLES order. Successful calls retain the existing measured
/// capture and comparison ticks. A terminal failure returns zero ticks because
/// no completed measurement is available, not because the failed work was free.
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
            rows: 8192,
            arena: 16384,
        }
    }
}
pub struct Captured {
    rows: CaptureRows<ccharacter::PortraitRow>,
    arena: Arena<engine::character_portrait_entry::Row>,
    terminal_failure: Option<&'static str>,
}
impl Captured {
    /// Allocation-free replacement while the original buffers are loaned to
    /// acquisition. Restore the original before capture, table views or growth.
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

    /// Run the existing hourly capture on its owning thread. The frame reset
    /// must precede dispatch, just as it precedes direct capture.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        let (matched, ticks) =
            self.capture(memory, context.base, context.game_state, &context.needed);
        CaptureReport { matched, ticks }
    }

    /// Complete an accepted failed request without reading or retrying native
    /// state. Keep observed growth demand and allocations behind a publication
    /// failure until recycling; partial rows must never reach table conversion.
    /// The caller stores this terminal report even though its ticks are unmeasured.
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
        let begin = qpc();
        self.reset();
        let first = ccharacter::walk_portraits(
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
            let second = ccharacter::walk_portraits(
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
                reason: "character_portraits changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::CharacterPortraits, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| CharacterPortraitRow {
                character: r.character,
                portraits: self.arena.first[r.portraits[0]..r.portraits[0] + r.portraits[1]]
                    .iter()
                    .map(|p| replay::rows::CharacterPortrait {
                        category: p.category,
                        size: p.size,
                        is_set: p.is_set,
                        kind: p.kind,
                        identity: p.identity.as_bytes().to_vec(),
                    })
                    .collect(),
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
    fn owned_failure_restores_allocations_and_requested_check() {
        let capacities = Capacities { rows: 2, arena: 3 };
        for needed in [false, true] {
            let successful = if needed { 0 } else { CONSISTENCY_TARGET };
            let mut frame = Captured::with_capacities(&capacities, &[successful]);
            let before = allocations(&frame);
            let bytes = frame.allocated_bytes();
            let mut owned = std::mem::replace(&mut frame, Captured::vacant());
            assert_eq!(frame.allocated_bytes(), 0);
            assert!(
                allocations(&frame)
                    .iter()
                    .all(|&(_, len, cap)| len == 0 && cap == 0)
            );
            assert_eq!(allocations(&owned), before);
            owned.rows.rows[0].portraits = [usize::MAX, 1];
            owned.rows.outcome = Outcome::Rows { len: 1 };
            owned.rows.mismatches = 7;
            owned.arena.len = 1;
            owned.arena.required = 19;
            let report = owned.fail_owned_capture(
                CaptureContext {
                    base: 0,
                    game_state: 0,
                    needed: [needed],
                },
                "owned character portraits failed",
            );
            assert_eq!(report.matched, [false]);
            assert_eq!(report.ticks, [0]); // No completed timing measurement.
            assert_eq!(allocations(&owned), before);
            assert_eq!(owned.rows.checked, needed);
            assert_eq!(owned.checked_mismatches(), [usize::from(needed)]);
            assert_eq!(owned.rows.outcome, Outcome::Rows { len: 1 });
            assert_eq!(
                owned.outcomes(),
                [Outcome::Unavailable {
                    reason: "owned character portraits failed"
                }]
            );
            assert_eq!((owned.arena.len, owned.arena.required), (1, 19));
            assert!(
                matches!(&owned.tables()[0], TableView::Unavailable { reason } if reason == "owned character portraits failed")
            );
            frame = owned;
            assert_eq!(frame.allocated_bytes(), bytes);
            assert_eq!(allocations(&frame), before);
            frame.reset();
            assert!(frame.terminal_failure.is_none());
            assert_eq!(frame.checked_mismatches(), [0]);
            assert!(!frame.rows.checked);
            assert_eq!((frame.arena.len, frame.arena.required), (0, 0));
            assert_eq!(allocations(&frame), before);
        }
    }

    #[test]
    fn owned_capture_reuses_direct_failure_reads_and_check_selection() {
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
            let before = allocations(&owned);
            let direct_memory = Unreadable::default();
            let owned_memory = Unreadable::default();
            let (matched, _) = direct.capture(
                &direct_memory,
                context.base,
                context.game_state,
                &context.needed,
            );
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
                assert_eq!(actual, matched);
                assert_eq!(owned.outcomes(), direct.outcomes());
                assert_eq!(owned.checked_mismatches(), direct.checked_mismatches());
                assert_eq!(owned.rows.checked, needed);
                assert_eq!(
                    (owned.arena.len, owned.arena.required),
                    (direct.arena.len, direct.arena.required)
                );
                assert_eq!(allocations(&owned), before);
            }
        }
    }

    #[test]
    fn restored_portrait_storage_keeps_pressure_and_retires_only_comparisons() {
        let initial = Capacities { rows: 2, arena: 2 };
        for (outcome, arena_len, required, expected_rows, expected_arena) in [
            (Outcome::Rows { len: 2 }, 2, 2, 4, 4),
            (
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 6,
                },
                1,
                9,
                8,
                16,
            ),
        ] {
            for needed in [false, true] {
                let mut capacities = initial;
                let mut expected_capacity = initial;
                let mut frame = Captured::with_capacities(&initial, &[0]);
                let mut owned = std::mem::replace(&mut frame, Captured::vacant());
                let mut expected = Captured::with_capacities(&initial, &[0]);
                for capture in [&mut owned, &mut expected] {
                    capture.rows.outcome = outcome;
                    capture.arena.len = arena_len;
                    capture.arena.required = arena_len;
                    assert_eq!(
                        capture.arena.compare(arena_len, required),
                        arena_len == required
                    );
                }
                let before = allocations(&owned);
                owned.fail_owned_capture(
                    CaptureContext {
                        base: 0,
                        game_state: 0,
                        needed: [needed],
                    },
                    "failure after observed demand",
                );
                assert_eq!(allocations(&owned), before);
                assert_eq!(owned.rows.outcome, outcome);
                assert_eq!(
                    (owned.arena.len, owned.arena.required),
                    (arena_len, required)
                );
                frame = owned;
                let successful = if needed { 0 } else { CONSISTENCY_TARGET };
                frame.grow(&mut capacities, &[successful]);
                expected.grow(&mut expected_capacity, &[successful]);
                assert!(capacities == expected_capacity);
                assert_eq!(
                    (capacities.rows, capacities.arena),
                    (expected_rows, expected_arena)
                );
                assert_eq!(
                    allocations(&frame).map(|(_, len, capacity)| (len, capacity)),
                    allocations(&expected).map(|(_, len, capacity)| (len, capacity))
                );
                assert_eq!(frame.allocated_bytes(), expected.allocated_bytes());
                assert_eq!(!frame.rows.second.is_empty(), needed);
                assert_eq!(!frame.arena.second.is_empty(), needed);
                assert_eq!(
                    frame.outcomes(),
                    [Outcome::Unavailable {
                        reason: "failure after observed demand"
                    }]
                );
                assert!(matches!(frame.tables()[0], TableView::Unavailable { .. }));
                let first = allocations(&frame);
                frame.reset();
                frame.grow(&mut capacities, &[CONSISTENCY_TARGET]);
                let retired = allocations(&frame);
                for index in [0, 2] {
                    assert_eq!(retired[index], first[index]);
                }
                for index in [1, 3] {
                    assert_eq!((retired[index].1, retired[index].2), (0, 0));
                }
            }
        }
    }
}
