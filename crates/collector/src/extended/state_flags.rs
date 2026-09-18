//! Hourly state flags, using the authenticated States census from this callback.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec},
};
use engine::{Memory, cstate, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{StateFlagRow, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::StateFlags];
// Metadata is emitted once by the combined States/StateFlags source group.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 2048 }
    }
}

pub struct Captured {
    rows: CaptureRows<cstate::FlagRow>,
    first: Vec<cstate::Source>,
    second: Vec<cstate::Source>,
    seen: [usize; 2],
    sources: [Outcome; 2],
    required: usize,
}

impl Default for Captured {
    fn default() -> Self {
        Self {
            rows: CaptureRows {
                rows: Vec::new(),
                second: Vec::new(),
                outcome: super::MISSING,
                mismatches: 0,
                checked: false,
            },
            first: Vec::new(),
            second: Vec::new(),
            seen: [0; 2],
            sources: [super::MISSING; 2],
            required: 0,
        }
    }
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
            first: vec![Default::default(); capacity.rows],
            second: comparison_rows(capacity.rows, successful[0]),
            seen: [0; 2],
            sources: [super::MISSING; 2],
            required: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.seen = [0; 2];
        self.sources = [super::MISSING; 2];
        self.required = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + (self.first.capacity() + self.second.capacity()) * size_of::<cstate::Source>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        grow_capacity(&mut capacity.rows, self.required);
        self.rows.grow(&mut capacity.rows, successful[0]);
        grow_vec(&mut self.first, capacity.rows);
        comparison(&mut self.second, capacity.rows, successful[0]);
    }
    pub fn visit(&mut self, source: cstate::Source, second: bool) {
        let pass = usize::from(second);
        let rows = if second {
            &mut self.second
        } else {
            &mut self.first
        };
        if let Some(row) = rows.get_mut(self.seen[pass]) {
            *row = source;
        }
        self.seen[pass] = self.seen[pass].saturating_add(1);
        self.required = self.required.max(self.seen[pass]);
    }
    pub fn finish(&mut self, outcome: Outcome, second: bool) {
        let pass = usize::from(second);
        let capacity = if second {
            self.second.len()
        } else {
            self.first.len()
        };
        self.sources[pass] = match outcome {
            Outcome::Unavailable { .. } => outcome,
            _ if outcome.len() != self.seen[pass] => Outcome::Unavailable {
                reason: "state visitor and source row count disagree",
            },
            _ => {
                let missing = match outcome {
                    Outcome::Truncated { unread_rows, .. } => unread_rows as usize,
                    _ => 0,
                };
                let total = outcome.len().saturating_add(missing);
                self.required = self.required.max(total);
                let stored = outcome.len().min(capacity);
                if total == stored {
                    Outcome::Rows { len: stored }
                } else {
                    super::truncated_source(stored, total.saturating_sub(stored))
                }
            }
        };
    }

    /// Accepted census failure is terminal for both passes, including a pass
    /// that finished before its peer panicked. Keep storage and growth demand;
    /// read_flags will reject these parents before touching native memory.
    pub(crate) fn fail_sources(&mut self, reason: &'static str) {
        self.release_sources();
        self.sources = [Outcome::Unavailable { reason }; 2];
        self.rows.outcome = Outcome::Unavailable { reason };
    }

    fn release_sources(&mut self) {
        for (sources, seen) in [
            (&mut self.first, self.seen[0]),
            (&mut self.second, self.seen[1]),
        ] {
            let len = sources.len().min(seen);
            sources[..len].fill(cstate::Source::default());
        }
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let begin = qpc();
        self.rows.outcome = cstate::read_flags(
            memory,
            base,
            state,
            &self.first,
            self.sources[0],
            &mut self.rows.rows,
        );
        self.rows.checked = needed[0];
        let matched = if needed[0] {
            let second = cstate::read_flags(
                memory,
                base,
                state,
                &self.second,
                self.sources[1],
                &mut self.rows.second,
            );
            self.rows.compare(second)
        } else {
            false
        };
        // Source pointers have no purpose after acquisition; conversion sees
        // owned numeric rows only, including after a failed read.
        self.release_sources();
        self.sources = [super::MISSING; 2];
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "state flags changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::StateFlags, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| StateFlagRow {
                state: r.state,
                demilitarized: r.demilitarized,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, memory::SliceMemory};
    use std::{
        cell::Cell,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    const STATE: usize = 0x1000;
    const SOURCE: cstate::Source = cstate::Source {
        pointer: 0x2000,
        id: 1,
    };

    fn memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        let root = STATE + cgame_state::STATES;
        bytes[root..root + 8].copy_from_slice(&0x1800_u64.to_le_bytes());
        bytes[root + 8..root + 12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[root + 12..root + 16].copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x1808..0x1810].copy_from_slice(&(SOURCE.pointer as u64).to_le_bytes());
        bytes[SOURCE.pointer..SOURCE.pointer + 8]
            .copy_from_slice(&(cstate::VTABLE_RVA as u64).to_le_bytes());
        bytes[SOURCE.pointer + cstate::ID..SOURCE.pointer + cstate::ID + 4]
            .copy_from_slice(&SOURCE.id.to_le_bytes());
        bytes[SOURCE.pointer + cstate::DEMILITARIZED] = 255;
        bytes
    }

    fn seed(captured: &mut Captured, second: bool) {
        captured.visit(SOURCE, second);
        captured.finish(Outcome::Rows { len: 1 }, second);
    }

    #[test]
    fn flags_independently_compare_native_bytes_and_release_callback_sources() {
        let bytes = memory();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 }, &[0]);
        seed(&mut captured, false);
        seed(&mut captured, true);
        let (matched, _) = captured.capture(&memory, 0, STATE, &[true]);
        assert_eq!(matched, [true]);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }]);
        assert_eq!(
            captured.rows.rows[0],
            cstate::FlagRow {
                state: 1,
                demilitarized: 255
            }
        );
        assert!(
            captured
                .first
                .iter()
                .chain(&captured.second)
                .all(|source| source.pointer == 0)
        );

        struct Changed<'a> {
            memory: SliceMemory<'a>,
            reads: Cell<usize>,
        }
        impl Memory for Changed<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if !self.memory.copy(address, destination) {
                    return false;
                }
                if address == SOURCE.pointer + cstate::DEMILITARIZED {
                    self.reads.set(self.reads.get() + 1);
                    if self.reads.get() > 1 {
                        destination[0] = 0;
                    }
                }
                true
            }
        }
        let memory = Changed {
            memory,
            reads: Cell::new(0),
        };
        captured.reset();
        seed(&mut captured, false);
        seed(&mut captured, true);
        let (matched, _) = captured.capture(&memory, 0, STATE, &[true]);
        assert_eq!(matched, [false]);
        assert_eq!(captured.checked_mismatches(), [1]);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "state flags changed during double read"
            }]
        );
    }

    #[test]
    fn flags_preserve_parent_faults_omissions_and_unsupplied_second_passes() {
        let bytes = memory();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities { rows: 1 }, &[0]);
        for (visits, parent, expected) in [
            (
                1,
                Outcome::Unavailable {
                    reason: "state census fault",
                },
                Outcome::Unavailable {
                    reason: "state census fault",
                },
            ),
            (
                0,
                Outcome::Rows { len: 1 },
                Outcome::Unavailable {
                    reason: "state visitor and source row count disagree",
                },
            ),
            (
                1,
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 3,
                },
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 3,
                },
            ),
            (
                2,
                Outcome::Rows { len: 2 },
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 1,
                },
            ),
        ] {
            captured.reset();
            for _ in 0..visits {
                captured.visit(SOURCE, false);
            }
            captured.finish(parent, false);
            captured.capture(&memory, 0, STATE, &[false]);
            assert_eq!(captured.outcomes(), [expected]);
        }
        captured.reset();
        seed(&mut captured, false);
        assert_eq!(captured.capture(&memory, 0, STATE, &[true]).0, [false]);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn state_flag_growth_and_second_source_retirement_follow_its_own_window() {
        let mut capacity = Capacities { rows: 1 };
        let mut captured = Captured::with_capacities(&capacity, &[CONSISTENCY_TARGET - 1]);
        captured.visit(SOURCE, false);
        captured.finish(
            Outcome::Truncated {
                len: 1,
                unread_rows: 3,
            },
            false,
        );
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET - 1]);
        assert!(capacity.rows >= 4);
        assert_eq!(captured.first.len(), capacity.rows);
        assert_eq!(captured.second.len(), capacity.rows);
        assert_eq!(captured.rows.second.len(), capacity.rows);
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(captured.second.is_empty());
        assert!(captured.rows.second.is_empty());
        assert!(!captured.first.is_empty());
    }

    fn layout<T>(rows: &Vec<T>) -> (usize, usize, usize) {
        (rows.as_ptr() as usize, rows.len(), rows.capacity())
    }

    fn visitor_layout(captured: &Captured) -> [(usize, usize, usize); 4] {
        [
            layout(&captured.rows.rows),
            layout(&captured.rows.second),
            layout(&captured.first),
            layout(&captured.second),
        ]
    }

    fn request_layout(request: &crate::acquisition_states::Request) -> [(usize, usize, usize); 6] {
        let [rows, second_rows, first, second] = visitor_layout(&request.visitor);
        [
            layout(&request.census.buffers.first),
            layout(&request.census.buffers.second),
            rows,
            second_rows,
            first,
            second,
        ]
    }

    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("unavailable state sources must fail before native reads");
        }
    }

    #[test]
    fn state_flags_default_is_allocation_free_and_take_keeps_visitor_metadata() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 }, &[0]);
        seed(&mut captured, false);
        seed(&mut captured, true);
        captured.finish(
            Outcome::Truncated {
                len: 1,
                unread_rows: 6,
            },
            false,
        );
        captured.rows.rows[0] = cstate::FlagRow {
            state: 1,
            demilitarized: 255,
        };
        captured.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 6,
        };
        captured.rows.checked = true;
        captured.rows.mismatches = 2;
        let pointers = visitor_layout(&captured);
        let sources = captured.sources;
        let moved = std::mem::take(&mut captured);
        assert_eq!(captured.allocated_bytes(), 0);
        assert!(
            visitor_layout(&captured)
                .iter()
                .all(|&(_, len, capacity)| len == 0 && capacity == 0)
        );
        assert_eq!(captured.seen, [0; 2]);
        assert_eq!(captured.sources, [super::super::MISSING; 2]);
        assert_eq!(captured.required, 0);
        assert!(!captured.rows.checked);
        assert_eq!(captured.rows.mismatches, 0);
        assert_eq!(captured.capture(&NoReads, 0, STATE, &[true]).0, [false]);
        captured = moved;
        assert_eq!(visitor_layout(&captured), pointers);
        assert_eq!(captured.sources, sources);
        assert_eq!(captured.seen, [1; 2]);
        assert_eq!(captured.required, 7);
        assert_eq!(captured.rows.rows[0].demilitarized, 255);
        assert_eq!(
            captured.rows.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 6
            }
        );
        assert!(captured.rows.checked);
        assert_eq!(captured.rows.mismatches, 2);
    }

    #[test]
    fn state_flags_terminal_source_failure_scrubs_both_passes_and_retains_growth_demand() {
        let mut capacity = Capacities { rows: 1 };
        let mut captured = Captured::with_capacities(&capacity, &[0]);
        seed(&mut captured, false);
        captured.finish(
            Outcome::Truncated {
                len: 1,
                unread_rows: 6,
            },
            false,
        );
        captured.visit(SOURCE, true);
        captured.visit(
            cstate::Source {
                id: 3,
                pointer: 0x4000,
            },
            true,
        );
        captured.finish(Outcome::Rows { len: 2 }, true);
        let pointers = visitor_layout(&captured);
        let seen = captured.seen;
        let reason = "state census worker failed";
        captured.fail_sources(reason);
        assert_eq!(visitor_layout(&captured), pointers);
        assert_eq!(captured.seen, seen);
        assert_eq!(captured.required, 7);
        assert_eq!(captured.sources, [Outcome::Unavailable { reason }; 2]);
        assert!(
            captured
                .first
                .iter()
                .chain(&captured.second)
                .all(|source| *source == cstate::Source::default())
        );
        assert_eq!(captured.capture(&NoReads, 0, STATE, &[true]).0, [false]);
        assert_eq!(captured.outcomes(), [Outcome::Unavailable { reason }]);
        assert_eq!(captured.required, 7);
        assert_eq!(visitor_layout(&captured), pointers);
        captured.grow(&mut capacity, &[0]);
        assert_eq!(capacity.rows, 8);
        assert_eq!(captured.first.len(), 8);
        assert_eq!(captured.second.len(), 8);
        assert_eq!(captured.rows.rows.len(), 8);
        assert_eq!(captured.rows.second.len(), 8);
    }

    #[test]
    fn state_flags_owned_census_keeps_core_row_source_and_flag_capacities_independent() {
        let mut bytes = memory();
        bytes.resize(0x5000, 0);
        let root = STATE + cgame_state::STATES;
        bytes[root + 8..root + 12].copy_from_slice(&4_u32.to_le_bytes());
        bytes[root + 12..root + 16].copy_from_slice(&4_u32.to_le_bytes());
        bytes[0x1818..0x1820].copy_from_slice(&0x4000_u64.to_le_bytes());
        bytes[0x4000..0x4008].copy_from_slice(&(cstate::VTABLE_RVA as u64).to_le_bytes());
        bytes[0x4000 + cstate::ID..0x4004 + cstate::ID].copy_from_slice(&3_u32.to_le_bytes());
        bytes[0x4000 + cstate::DEMILITARIZED] = 2;
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        for (core_capacity, source_capacity, flag_capacity) in
            [(1, 2, 2), (2, 1, 2), (2, 2, 1), (0, 2, 2)]
        {
            let mut visitor = Captured::with_capacities(&Capacities { rows: 2 }, &[0]);
            visitor.first.truncate(source_capacity);
            visitor.second.truncate(source_capacity);
            visitor.rows.rows.truncate(flag_capacity);
            visitor.rows.second.truncate(flag_capacity);
            let mut request = crate::acquisition_states::Request {
                census: crate::acquisition_gather::Request {
                    base: 0,
                    game_state: STATE,
                    needed: true,
                    buffers: crate::acquisition_gather::Buffers {
                        first: vec![cstate::Row::default(); core_capacity],
                        second: vec![cstate::Row::default(); core_capacity],
                    },
                },
                flags_needed: true,
                visitor,
            };
            let report = request.run(&memory);
            assert_eq!(report.census.mismatches, 0);
            assert_eq!(request.visitor.required, 2);
            let len = core_capacity.min(source_capacity).min(flag_capacity);
            assert_eq!(
                request.visitor.capture(&memory, 0, STATE, &[true]).0,
                [false]
            );
            assert_eq!(
                request.visitor.outcomes(),
                [Outcome::Truncated {
                    len,
                    unread_rows: (2 - len) as u32
                }]
            );
            assert_eq!(request.visitor.required, 2);
            assert_eq!(report.census.result.len(), core_capacity);
            assert_eq!(
                matches!(report.census.result, Outcome::Rows { len: 2 }),
                core_capacity == 2
            );
            assert!(
                request
                    .visitor
                    .first
                    .iter()
                    .chain(&request.visitor.second)
                    .all(|source| source.pointer == 0)
            );
        }
    }

    #[test]
    fn state_flags_terminal_pool_completion_restores_all_six_buffers_without_native_retry() {
        use crate::{
            acquisition::{Failure, Pool},
            acquisition_states::{Report, Request},
        };

        struct PanicAtRoot<'a> {
            bytes: &'a [u8],
            roots: Cell<usize>,
            panic_at: usize,
            calls: &'a AtomicUsize,
        }
        impl Memory for PanicAtRoot<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.fetch_add(1, Ordering::Relaxed);
                if address == STATE + cgame_state::STATES {
                    let count = self.roots.get() + 1;
                    self.roots.set(count);
                    assert_ne!(count, self.panic_at, "synthetic state census panic");
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }

        let mut frame = crate::sampling::Captured::new();
        for needed in [[false, false], [true, false], [false, true], [true, true]] {
            for canceled in [false, true] {
                frame
                    .extended
                    .restore_state_visitor(Captured::with_capacities(
                        &Capacities { rows: 1 },
                        &[if needed[1] { 0 } else { CONSISTENCY_TARGET }],
                    ));
                let request = frame.take_states_request(0, STATE, needed[0], needed[1]);
                let buffers = request_layout(&request);
                let vacant = frame.take_states_request(0, STATE, needed[0], needed[1]);
                assert!(
                    request_layout(&vacant)
                        .iter()
                        .all(|&(_, len, capacity)| len == 0 && capacity == 0)
                );
                let calls = Arc::new(AtomicUsize::new(0));
                let runner_calls = Arc::clone(&calls);
                let bytes = memory();
                let pool = Pool::start(0, 1, move |request: &mut Request| {
                    request.run(&PanicAtRoot {
                        bytes: &bytes,
                        roots: Cell::new(0),
                        panic_at: if needed[0] || needed[1] { 4 } else { 2 },
                        calls: &runner_calls,
                    })
                })
                .unwrap();
                let mut pending = Some(request);
                // SAFETY: the fixture and output storage remain owned through collection.
                let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
                if canceled {
                    pool.shutdown().unwrap();
                }
                let completed = pool.finish(ticket).unwrap();
                assert!(
                    matches!(completed.report, Err(failure) if failure == if canceled { Failure::Stopped } else { Failure::Panicked })
                );
                let demand = completed.request.visitor.required;
                assert_eq!(demand, usize::from(!canceled));
                let calls_before = calls.load(Ordering::Relaxed);
                let reason = "terminal state census failure";
                let report = frame.restore_states_request(
                    completed.request,
                    Report::default(),
                    Some(reason),
                );
                assert_eq!(frame.states_result, Outcome::Unavailable { reason });
                assert_eq!(frame.consistency_checked[1], needed[0]);
                assert_eq!(frame.states_mismatches, usize::from(needed[0]));
                assert_eq!(report.ticks, 0);
                let mut restored = frame.take_states_request(0, STATE, needed[0], needed[1]);
                assert_eq!(request_layout(&restored), buffers);
                assert_eq!(
                    restored.visitor.sources,
                    [Outcome::Unavailable { reason }; 2]
                );
                assert_eq!(restored.visitor.required, demand);
                assert!(
                    restored
                        .visitor
                        .first
                        .iter()
                        .chain(&restored.visitor.second)
                        .all(|source| *source == cstate::Source::default())
                );
                assert_eq!(
                    restored.visitor.capture(&NoReads, 0, STATE, &[needed[1]]).0,
                    [false]
                );
                assert_eq!(
                    restored.visitor.outcomes(),
                    [Outcome::Unavailable { reason }]
                );
                assert_eq!(request_layout(&restored), buffers);
                assert_eq!(restored.visitor.required, demand);
                assert_eq!(calls.load(Ordering::Relaxed), calls_before);
                assert_eq!(pool.calibration().uncollected, 0);
                frame.restore_states_request(restored, report, None);
            }
        }
    }

    #[test]
    fn state_flags_successful_states_restore_preserves_timing_and_core_mismatch_independence() {
        struct ChangedOwner<'a> {
            bytes: &'a [u8],
            owner_reads: Cell<usize>,
        }
        impl Memory for ChangedOwner<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let copied = SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output);
                if address == SOURCE.pointer + cstate::OWNER {
                    let reads = self.owner_reads.get() + 1;
                    self.owner_reads.set(reads);
                    if reads == 2 {
                        output[0] ^= 1;
                    }
                }
                copied
            }
        }
        let mut frame = crate::sampling::Captured::new();
        let bytes = memory();
        for core_needed in [false, true] {
            frame.extended.reset();
            let memory = ChangedOwner {
                bytes: &bytes,
                owner_reads: Cell::new(0),
            };
            let mut request = frame.take_states_request(0, STATE, core_needed, true);
            let buffers = request_layout(&request);
            let mut report = request.run(&memory);
            assert_eq!(report.census.mismatches, usize::from(core_needed));
            // A distinct synthetic cost verifies restoration returns the runner's value.
            report.ticks = 137;
            let restored = frame.restore_states_request(request, report, None);
            assert_eq!(restored.ticks, 137);
            assert_eq!(frame.states_result, Outcome::Rows { len: 1 });
            assert_eq!(frame.states_mismatches, usize::from(core_needed));
            assert_eq!(frame.consistency_checked[1], core_needed);
            let mut request = frame.take_states_request(0, STATE, core_needed, true);
            assert_eq!(request_layout(&request), buffers);
            assert_eq!(request.visitor.sources, [Outcome::Rows { len: 1 }; 2]);
            assert_eq!(
                request.visitor.capture(&memory, 0, STATE, &[true]).0,
                [true]
            );
            assert_eq!(request.visitor.outcomes(), [Outcome::Rows { len: 1 }]);
            assert_eq!(request_layout(&request), buffers);
            assert!(
                request
                    .visitor
                    .first
                    .iter()
                    .chain(&request.visitor.second)
                    .all(|source| source.pointer == 0)
            );
            frame.restore_states_request(request, restored, None);
        }
    }
}
