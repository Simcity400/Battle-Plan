use collector_primitives::{
    clock::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use collector_projection::table_conversion::table;
use engine_allocation::{copy_vec, default_vec, owned_vec};
use engine_native::{Memory, cstate, walk::Outcome};
use replay_data::{rows::StateFlagRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::mem::size_of;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::StateFlags];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });

impl Capacities {
    pub fn raise_for_states(&mut self, states: usize) {
        self.rows = self.rows.max(states);
    }
}

pub struct Captured {
    rows: CaptureRows<cstate::FlagRow>,
    pub first: Vec<cstate::Source>,
    seen: usize,
    pub source: Outcome,
    pub required: usize,
    acquired_ticks: Option<u64>,
    source_failure: Option<&'static str>,
}

impl Default for Captured {
    fn default() -> Self {
        let mut rows = CaptureRows::new(0);
        rows.outcome = collector_extended_support::outcomes::MISSING;
        Self {
            rows,
            first: Vec::new(),
            seen: 0,
            source: collector_extended_support::outcomes::MISSING,
            required: 0,
            acquired_ticks: None,
            source_failure: None,
        }
    }
}

impl Captured {
    #[cfg(any(test, feature = "test-support"))]
    pub fn install_border_fixture(&mut self, rows: Vec<cstate::FlagRow>, outcome: Outcome) {
        self.rows.rows = rows;
        self.rows.outcome = outcome;
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            first: owned_vec(default_vec(capacity.rows)),
            seen: 0,
            source: collector_extended_support::outcomes::MISSING,
            required: 0,
            acquired_ticks: None,
            source_failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.seen = 0;
        self.source = collector_extended_support::outcomes::MISSING;
        self.required = 0;
        self.acquired_ticks = None;
        self.source_failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.first.capacity() * size_of::<cstate::Source>()
            + self
                .rows
                .rows
                .iter()
                .map(|row| row.border_conflict_countries.capacity() * size_of::<[i32; 2]>())
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required);
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.first, capacity.rows);
    }
    pub fn visit(&mut self, source: cstate::Source) {
        if self.source_failure.is_none() {
            match self
                .seen
                .checked_add(1)
                .ok_or("state visitor demand overflow")
                .and_then(|required| {
                    engine_native::walk::Buffer::Growing(&mut self.first).ensure(required)
                }) {
                Ok(_) => {}
                Err(reason) => self.source_failure = Some(reason),
            }
        }
        if let Some(row) = self.first.get_mut(self.seen) {
            *row = source;
        }
        self.seen = self.seen.saturating_add(1);
        self.required = self.required.max(self.seen);
    }

    pub fn finish(&mut self, outcome: Outcome) {
        if let Some(reason) = self.source_failure {
            self.source = Outcome::Unavailable { reason };
            return;
        }
        let capacity = self.first.len();
        self.source = match outcome {
            Outcome::Unavailable { .. } => outcome,
            _ if outcome.len() != self.seen => Outcome::Unavailable {
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
                    collector_extended_support::outcomes::truncated_source(
                        stored,
                        total.saturating_sub(stored),
                    )
                }
            }
        };
    }

    pub fn completed_sources(&self) -> (&[cstate::Source], Outcome, usize) {
        (&self.first[..self.source.len()], self.source, self.required)
    }

    pub fn fail_sources(&mut self, reason: &'static str) {
        self.release_sources();
        self.source = Outcome::Unavailable { reason };
        self.rows.outcome = Outcome::Unavailable { reason };
        self.acquired_ticks = Some(0);
    }

    fn release_sources(&mut self) {
        let len = self.first.len().min(self.seen);
        self.first[..len].fill(cstate::Source::default());
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        if let Some(ticks) = self.acquired_ticks.take() {
            return [ticks];
        }
        self.acquire(memory, base, state)
    }

    pub fn acquire_owned(&mut self, memory: &impl Memory, base: usize, state: usize) {
        if let Err(reason) =
                engine_native::walk::Buffer::Growing(&mut self.rows.rows).ensure(self.source.len())
        {
            self.fail_sources(reason);
            return;
        }
        self.acquired_ticks = Some(self.acquire(memory, base, state)[0]);
    }

    fn acquire(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome = cstate::read_flags(
            memory,
            base,
            state,
            &self.first,
            self.source,
            &mut self.rows.rows,
        );
        self.release_sources();
        self.source = collector_extended_support::outcomes::MISSING;
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn rows(&self) -> Result<&[cstate::FlagRow], &'static str> {
        match self.rows.outcome {
            Outcome::Rows { len } => self
                .rows
                .rows
                .get(..len)
                .ok_or("state flag rows exceed buffer"),
            Outcome::Unavailable { reason } => Err(reason),
            Outcome::Truncated { .. } => Err("border demand state flags truncated"),
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::StateFlags, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| StateFlagRow {
                state: r.state,
                demilitarized: r.demilitarized,
                border_conflict_active: replay_data::rows::Recorded::Value(
                    r.border_conflict_active,
                ),
                border_conflict_countries: replay_data::rows::Recorded::Value(owned_vec(copy_vec(
                    &r.border_conflict_countries,
                ))),
            },
        )]
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use engine_native::cgame_state;
    #[cfg(test)]
    use crate::acquisition_states::tests::Movement;
    #[cfg(test)]
    use engine_native::memory::SliceMemory;
    pub const STATE: usize = 0x1000;
    const SOURCE: cstate::Source = cstate::Source {
        pointer: 0x2000,
        id: 1,
    };

    pub fn memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        let root = STATE + cgame_state::STATES;
        bytes[root..root + 8].copy_from_slice(&0x1800_u64.to_le_bytes());
        bytes[root + 8..root + 12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[root + 12..root + 16].copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x1808..0x1810].copy_from_slice(&(SOURCE.pointer as u64).to_le_bytes());
        bytes[SOURCE.pointer..SOURCE.pointer + 8]
            .copy_from_slice(&(cstate::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[SOURCE.pointer + cstate::ID..SOURCE.pointer + cstate::ID + 4]
            .copy_from_slice(&SOURCE.id.to_le_bytes());
        bytes[SOURCE.pointer + cstate::DEMILITARIZED] = 255;
        empty_names(&mut bytes, SOURCE.pointer);
        bytes
    }

    fn empty_names(bytes: &mut [u8], state: usize) {
        for field in [cstate::PRIMARY_DISPLAY_NAME, cstate::SECONDARY_DISPLAY_NAME] {
            let capacity = state + field + engine_native::msvc_string::CAPACITY;
            bytes[capacity..capacity + 8].copy_from_slice(&15_u64.to_le_bytes());
        }
    }

    #[cfg(test)]
    fn seed(captured: &mut Captured) {
        captured.visit(SOURCE);
        captured.finish(Outcome::Rows { len: 1 });
    }

    #[test]
    fn flags_preserve_primary_native_bytes_and_release_callback_sources() {
        let bytes = memory();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 });
        seed(&mut captured);
        captured.capture(&memory, 0, STATE);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }]);
        assert_eq!(
            captured.rows.rows[0],
            cstate::FlagRow {
                border_conflict_active: 0,
                border_conflict_countries: Vec::new(),
                state: 1,
                demilitarized: 255
            }
        );
        assert!(captured.first.iter().all(|source| source.pointer == 0));
    }

    #[test]
    fn flags_preserve_parent_faults_and_omissions() {
        let bytes = memory();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities { rows: 1 });
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
                captured.visit(SOURCE);
            }
            captured.finish(parent);
            captured.capture(&memory, 0, STATE);
            assert_eq!(captured.outcomes(), [expected]);
        }
    }

    #[test]
    fn state_flag_growth_preserves_primary_source_storage() {
        let mut capacity = Capacities { rows: 1 };
        let mut captured = Captured::with_capacities(&capacity);
        captured.visit(SOURCE);
        captured.finish(Outcome::Truncated {
            len: 1,
            unread_rows: 3,
        });
        captured.grow(&mut capacity);
        assert!(capacity.rows >= 4);
        assert_eq!(captured.first.len(), capacity.rows);
        assert_eq!(captured.rows.rows.len(), capacity.rows);
        assert_eq!(captured.first[0], SOURCE);
    }

    fn layout<T>(rows: &Vec<T>) -> (usize, usize, usize) {
        (rows.as_ptr() as usize, rows.len(), rows.capacity())
    }

    fn visitor_layout(captured: &Captured) -> [(usize, usize, usize); 2] {
        [layout(&captured.rows.rows), layout(&captured.first)]
    }

    pub fn request_layout(
        request: &crate::acquisition_states::Request,
    ) -> [(usize, usize, usize); 3] {
        let [rows, sources] = visitor_layout(&request.visitor);
        [layout(&request.census.payload), rows, sources]
    }

    pub struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("unavailable state sources must fail before native reads");
        }
    }

    #[test]
    fn state_flags_default_is_allocation_free_and_take_keeps_visitor_metadata() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 });
        seed(&mut captured);
        captured.finish(Outcome::Truncated {
            len: 1,
            unread_rows: 6,
        });
        captured.rows.rows[0] = cstate::FlagRow {
            border_conflict_active: 0,
            border_conflict_countries: Vec::new(),
            state: 1,
            demilitarized: 255,
        };
        captured.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 6,
        };
        let pointers = visitor_layout(&captured);
        let source = captured.source;
        let moved = std::mem::take(&mut captured);
        assert_eq!(captured.allocated_bytes(), 0);
        assert!(
            visitor_layout(&captured)
                .iter()
                .all(|&(_, len, capacity)| len == 0 && capacity == 0)
        );
        assert_eq!(captured.seen, 0);
        assert_eq!(
            captured.source,
            collector_extended_support::outcomes::MISSING
        );
        assert_eq!(captured.required, 0);
        captured.capture(&NoReads, 0, STATE);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured = moved;
        assert_eq!(visitor_layout(&captured), pointers);
        assert_eq!(captured.source, source);
        assert_eq!(captured.seen, 1);
        assert_eq!(captured.required, 7);
        assert_eq!(captured.rows.rows[0].demilitarized, 255);
        assert_eq!(
            captured.rows.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 6
            }
        );
    }

    #[test]
    fn state_flags_source_failure_scrubs_pointers_and_retains_growth_demand() {
        let mut capacity = Capacities { rows: 1 };
        let mut captured = Captured::with_capacities(&capacity);
        seed(&mut captured);
        captured.finish(Outcome::Truncated {
            len: 1,
            unread_rows: 6,
        });
        let pointers = visitor_layout(&captured);
        let seen = captured.seen;
        let reason = "state census worker failed";
        captured.fail_sources(reason);
        assert_eq!(visitor_layout(&captured), pointers);
        assert_eq!(captured.seen, seen);
        assert_eq!(captured.required, 7);
        assert_eq!(captured.source, Outcome::Unavailable { reason });
        assert!(
            captured
                .first
                .iter()
                .all(|source| *source == cstate::Source::default())
        );
        captured.capture(&NoReads, 0, STATE);
        assert_eq!(captured.outcomes(), [Outcome::Unavailable { reason }]);
        assert_eq!(captured.required, 7);
        assert_eq!(visitor_layout(&captured), pointers);
        captured.grow(&mut capacity);
        assert_eq!(capacity.rows, 8);
        assert_eq!(captured.first.len(), 8);
        assert_eq!(captured.rows.rows.len(), 8);
    }

    #[test]
    fn state_flags_owned_census_grows_core_row_source_and_flag_storage_independently() {
        let mut bytes = memory();
        bytes.resize(0x5000, 0);
        let root = STATE + cgame_state::STATES;
        bytes[root + 8..root + 12].copy_from_slice(&4_u32.to_le_bytes());
        bytes[root + 12..root + 16].copy_from_slice(&4_u32.to_le_bytes());
        bytes[0x1818..0x1820].copy_from_slice(&0x4000_u64.to_le_bytes());
        bytes[0x4000..0x4008].copy_from_slice(&(cstate::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x4000 + cstate::ID..0x4004 + cstate::ID].copy_from_slice(&3_u32.to_le_bytes());
        bytes[0x4000 + cstate::DEMILITARIZED] = 2;
        empty_names(&mut bytes, 0x4000);
        let memory = Movement(SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        for (core_capacity, source_capacity, flag_capacity) in
            [(1, 2, 2), (2, 1, 2), (2, 2, 1), (0, 2, 2)]
        {
            let mut visitor = Captured::with_capacities(&Capacities { rows: 2 });
            visitor.first.truncate(source_capacity);
            visitor.rows.rows.truncate(flag_capacity);
            let mut request = crate::acquisition_states::Request {
                census: crate::acquisition_gather::Request {
                    base: 0,
                    game_state: STATE,
                    payload: vec![cstate::Row::default(); core_capacity],
                },
                names: crate::acquisition_states::Names::take(&mut vec![0; 64]),
                visitor,
                inputs: crate::acquisition_states::Inputs::with_capacity(core_capacity).unwrap(),
                store: None,
                countries: None,
                occupation_store: None,
            };
            let report = request.run(&memory);
            assert_eq!(request.visitor.required, 2);
            request.visitor.capture(&memory, 0, STATE);
            assert_eq!(request.visitor.outcomes(), [Outcome::Rows { len: 2 }]);
            assert_eq!(request.visitor.required, 2);
            assert_eq!(report.census.result, Outcome::Rows { len: 2 });
            assert!(
                request
                    .visitor
                    .first
                    .iter()
                    .all(|source| source.pointer == 0)
            );
        }
    }

    #[cfg(test)]
    struct FlagPopulation(usize);
    #[cfg(test)]
    impl Memory for FlagPopulation {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            const POINTERS: usize = 0x100_000;
            const OBJECTS: usize = 0x1000_0000;
            for (offset, byte) in output.iter_mut().enumerate() {
                let at = address + offset;
                let root = STATE + cgame_state::STATES;
                *byte = if (root..root + 16).contains(&at) {
                    let mut header = [0u8; 16];
                    header[..8].copy_from_slice(&(POINTERS as u64).to_le_bytes());
                    header[8..12].copy_from_slice(&(self.0 as u32 + 1).to_le_bytes());
                    header[12..].copy_from_slice(&(self.0 as u32 + 1).to_le_bytes());
                    header[at - root]
                } else if (POINTERS..POINTERS + (self.0 + 1) * 8).contains(&at) {
                    let slot = (at - POINTERS) / 8;
                    let pointer = if slot == 0 {
                        0
                    } else {
                        OBJECTS + slot * 0x1000
                    };
                    (pointer as u64).to_le_bytes()[(at - POINTERS) % 8]
                } else if (OBJECTS + 0x1000..OBJECTS + (self.0 + 1) * 0x1000).contains(&at) {
                    let slot = (at - OBJECTS) / 0x1000;
                    let within = (at - OBJECTS) % 0x1000;
                    if within < 8 {
                        (cstate::VTABLE_RVA.0 as u64).to_le_bytes()[within]
                    } else if (cstate::ID..cstate::ID + 4).contains(&within) {
                        (slot as u32).to_le_bytes()[within - cstate::ID]
                    } else if within == cstate::DEMILITARIZED {
                        (slot % 2) as u8
                    } else {
                        0
                    }
                } else {
                    return false;
                };
            }
            true
        }
    }

    #[test]
    fn states_worker_retains_full_census_before_first_and_later_flags_publication() {
        let mut captured = Captured::default();
        for count in [5_000, 11_000] {
            captured.reset();
            for id in 1..=count {
                captured.visit(cstate::Source {
                    pointer: 0x1000_0000 + id * 0x1000,
                    id: id as u32,
                });
            }
            captured.finish(Outcome::Rows { len: count });
            assert_eq!(captured.completed_sources().1, Outcome::Rows { len: count });
            captured.acquire_owned(&FlagPopulation(count), 0, STATE);
            assert_eq!(captured.outcomes(), [Outcome::Rows { len: count }]);
            assert_eq!(captured.rows().unwrap()[count - 1].state, count as u32);
            assert!(
                captured
                    .first
                    .iter()
                    .all(|source| *source == cstate::Source::default())
            );
        }
    }
}
