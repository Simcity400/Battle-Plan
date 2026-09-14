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
        for (sources, seen) in [
            (&mut self.first, self.seen[0]),
            (&mut self.second, self.seen[1]),
        ] {
            let len = sources.len().min(seen);
            sources[..len].fill(cstate::Source::default());
        }
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
    use std::cell::Cell;

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
}
