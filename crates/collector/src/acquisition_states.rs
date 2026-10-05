//! Owned States census and its callback-scoped StateFlags visitor.
//! Flags and active border trees use this worker's authenticated census;
//! assembly later consumes their already acquired timing and owned rows.

use crate::{acquisition_gather, extended::state_flags, sampling};
use engine::{Memory, cstate, walk::Outcome};

pub(crate) struct Request {
    pub census: acquisition_gather::Request<Vec<cstate::Row>>,
    pub names: Names,
    pub visitor: state_flags::Captured,
}

/// The frame's state display-name arena, owned by the request while it runs.
#[derive(Default)]
pub(crate) struct Names {
    bytes: Vec<u8>,
    len: usize,
    required: usize,
}

impl Names {
    pub(crate) fn take(bytes: &mut Vec<u8>) -> Self {
        Self {
            bytes: std::mem::take(bytes),
            len: 0,
            required: 0,
        }
    }

    /// Returns the bytes copied and the bytes every row needs.
    pub(crate) fn restore(self, bytes: &mut Vec<u8>) -> (usize, usize) {
        *bytes = self.bytes;
        (self.len, self.required)
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub census: acquisition_gather::Report,
    pub ticks: i64,
}
impl Report {
    pub(crate) fn failed(reason: &'static str) -> Self {
        Self {
            census: acquisition_gather::Report {
                result: Outcome::Unavailable { reason },
            },
            ticks: 0,
        }
    }
}
impl Request {
    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        let _reads = crate::read_accounting::table(replay::rows::TableId::States);
        let Self {
            census,
            names,
            visitor,
        } = self;
        let (base, game_state) = (census.base, census.game_state);
        let begin = sampling::qpc();
        let report = census.run(|rows| {
            let outcome = cstate::walk_with(memory, base, game_state, rows, |source| {
                visitor.visit(source)
            });
            visitor.finish(outcome);
            let outcome =
                cstate::read_movement_infrastructure(memory, base, game_state, outcome, rows);
            let walk = cstate::read_names(memory, game_state, outcome, rows, &mut names.bytes);
            names.len = walk.bytes_len;
            names.required = walk.required_bytes;
            walk.outcome
        });
        let ticks = sampling::qpc() - begin;
        visitor.acquire_owned(memory, base, game_state);
        Report {
            census: report,
            ticks,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::acquisition::Pool;
    use engine::{cbuilding_database, cbuilding_template, cgame_state, memory::SliceMemory};
    use std::cell::RefCell;

    const STATE: usize = 0x1000;
    const ROOT: usize = 0x1800;
    const FIRST: usize = 0x2000;
    const LAST: usize = 0x4000;

    fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
        bytes[at..at + N].copy_from_slice(&value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        let root = STATE + cgame_state::STATES;
        put(&mut bytes, root, (ROOT as u64).to_le_bytes());
        put(&mut bytes, root + 8, 5_u32.to_le_bytes());
        put(&mut bytes, root + 12, 4_u32.to_le_bytes());
        // Reserved slot zero, a null hole, and poisoned spare capacity.
        for (slot, pointer) in [
            (0, u64::MAX),
            (1, FIRST as u64),
            (3, LAST as u64),
            (4, u64::MAX),
        ] {
            put(&mut bytes, ROOT + slot * 8, pointer.to_le_bytes());
        }
        for (pointer, row, flag) in [
            (
                FIRST,
                cstate::Row {
                    id: 1,
                    owner: 0,
                    controller: 0xffff_fffe,
                    ..Default::default()
                },
                2,
            ),
            (
                LAST,
                cstate::Row {
                    id: 3,
                    owner: 0x8000_0001,
                    controller: 0,
                    ..Default::default()
                },
                255,
            ),
        ] {
            put(
                &mut bytes,
                pointer,
                (cstate::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + cstate::ID, row.id.to_le_bytes());
            put(&mut bytes, pointer + cstate::OWNER, row.owner.to_le_bytes());
            put(
                &mut bytes,
                pointer + cstate::CONTROLLER,
                row.controller.to_le_bytes(),
            );
            bytes[pointer + cstate::DEMILITARIZED] = flag;
            for field in [cstate::PRIMARY_DISPLAY_NAME, cstate::SECONDARY_DISPLAY_NAME] {
                put(
                    &mut bytes,
                    pointer + field + engine::msvc_string::CAPACITY,
                    15_u64.to_le_bytes(),
                );
            }
        }
        bytes
    }

    // Synthetic building database and infrastructure definition, beside the
    // module globals rather than inside the dense fixture's address range.
    const DATABASE: usize = 0x0331_0000;
    const DEFINITION: usize = 0x0331_1000;
    const MAXIMUM: i32 = 5;
    // The per-level define at base+0x0331_aac0, fixed5.
    const PER_LEVEL: i64 = -10_000;
    // A disabled template lookup keeps every healthy level at zero, so the term
    // is maximum * per level. Nonzero, so a denied pass cannot be mistaken for it.
    const MOVEMENT_INFRASTRUCTURE: i64 = MAXIMUM as i64 * PER_LEVEL;

    /// The movement globals as (address, eight little-endian bytes).
    fn movement_globals() -> [(usize, [u8; 8]); 5] {
        [
            (
                cbuilding_database::SINGLETON_RVA,
                (DATABASE as u64).to_le_bytes(),
            ),
            (DATABASE + 0x3b0, (DEFINITION as u64).to_le_bytes()),
            (
                DEFINITION + cbuilding_template::SCOPE,
                i64::from(MAXIMUM).to_le_bytes(),
            ),
            (
                DEFINITION + cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
                [0; 8],
            ),
            (0x0331_aac0, PER_LEVEL.to_le_bytes()),
        ]
    }

    /// The movement pass chases module globals at RVAs far above any dense
    /// census fixture, so this overlays them on the fixture memory a test
    /// already uses. Wrapped by tracing memories, so its reads are observed.
    pub(crate) struct Movement<M>(pub(crate) M);

    impl<M: Memory> Memory for Movement<M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (at, bytes) in movement_globals() {
                let Some(start) = address.checked_sub(at) else {
                    continue;
                };
                let Some(end) = start
                    .checked_add(destination.len())
                    .filter(|end| *end <= bytes.len())
                else {
                    continue;
                };
                destination.copy_from_slice(&bytes[start..end]);
                return true;
            }
            self.0.copy(address, destination)
        }
    }

    fn request(capacity: usize) -> Request {
        Request {
            census: acquisition_gather::Request {
                base: 0,
                game_state: STATE,
                payload: vec![cstate::Row::default(); capacity],
            },
            names: Names {
                bytes: vec![0; 64],
                len: 0,
                required: 0,
            },
            visitor: state_flags::Captured::with_capacities(&Default::default()),
        }
    }

    struct Traced<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl<'a> Traced<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                bytes,
                calls: RefCell::default(),
                fault: None,
            }
        }
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let index = self.calls.borrow().len();
            self.calls.borrow_mut().push((address, output.len()));
            if self.fault == Some(index) {
                return false;
            }
            Movement(SliceMemory {
                base: 0,
                bytes: self.bytes,
            })
            .copy(address, output)
        }
    }

    fn serial(request: &mut Request, memory: &impl Memory) -> Outcome {
        let outcome = cstate::walk_with(memory, 0, STATE, &mut request.census.payload, |source| {
            request.visitor.visit(source)
        });
        request.visitor.finish(outcome);
        let outcome = cstate::read_movement_infrastructure(
            memory,
            0,
            STATE,
            outcome,
            &mut request.census.payload,
        );
        let names = cstate::read_names(
            memory,
            STATE,
            outcome,
            &mut request.census.payload,
            &mut request.names.bytes,
        );
        // Run acquires the flags and active border trees after its census.
        request.visitor.acquire_owned(memory, 0, STATE);
        names.outcome
    }

    #[test]
    fn one_states_census_matches_serial_rows_and_acquires_flags_once() {
        let bytes = fixture();
        for capacity in [0, 1, 2] {
            let mut actual = request(capacity);
            let mut reference = request(capacity);
            let memory = Traced::new(&bytes);
            let expected = Traced::new(&bytes);
            let report = actual.run(&memory);
            let outcome = serial(&mut reference, &expected);
            assert_eq!(report.census.result, outcome);
            assert_eq!(actual.census.payload, reference.census.payload);
            assert_eq!(*memory.calls.borrow(), *expected.calls.borrow());
            // The visitor's capture consumes the rows run already acquired.
            let acquired = memory.calls.borrow().len();
            actual.visitor.capture(&memory, 0, STATE);
            assert_eq!(memory.calls.borrow().len(), acquired);
            reference.visitor.capture(&expected, 0, STATE);
            assert_eq!(actual.visitor.tables(), reference.visitor.tables());
            assert_eq!(*memory.calls.borrow(), *expected.calls.borrow());
            if capacity == 2 {
                assert_eq!(report.census.result, Outcome::Rows { len: 2 });
                assert!(
                    actual
                        .census
                        .payload
                        .iter()
                        .all(|row| row.movement_infrastructure == MOVEMENT_INFRASTRUCTURE)
                );
                assert_eq!(actual.visitor.outcomes(), [Outcome::Rows { len: 2 }]);
                assert_eq!(
                    memory
                        .calls
                        .borrow()
                        .iter()
                        .filter(|(at, _)| *at == FIRST + cstate::DEMILITARIZED)
                        .count(),
                    1
                );
                assert_eq!(
                    memory
                        .calls
                        .borrow()
                        .iter()
                        .filter(|(at, _)| *at == LAST + cstate::DEMILITARIZED)
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn every_primary_states_fault_preserves_the_single_serial_trace() {
        let bytes = fixture();
        let healthy = Traced::new(&bytes);
        request(2).run(&healthy);
        let count = healthy.calls.borrow().len();
        for fault in 0..count {
            let mut memory = Traced::new(&bytes);
            memory.fault = Some(fault);
            let mut expected = Traced::new(&bytes);
            expected.fault = Some(fault);
            let mut actual = request(2);
            let mut reference = request(2);
            let report = actual.run(&memory);
            let outcome = serial(&mut reference, &expected);
            assert_eq!(report.census.result, outcome, "fault {fault}");
            // A census fault fails the census; a later flag fault fails only
            // the flags run acquired after it.
            assert!(
                matches!(report.census.result, Outcome::Unavailable { .. })
                    || matches!(actual.visitor.outcomes(), [Outcome::Unavailable { .. }]),
                "fault {fault}"
            );
            assert_eq!(actual.census.payload, reference.census.payload);
            assert_eq!(*memory.calls.borrow(), *expected.calls.borrow());
            actual.visitor.capture(&memory, 0, STATE);
            reference.visitor.capture(&expected, 0, STATE);
            assert_eq!(actual.visitor.tables(), reference.visitor.tables());
            assert_eq!(*memory.calls.borrow(), *expected.calls.borrow());
        }
    }

    #[test]
    fn states_pool_returns_single_census_storage_and_visitor() {
        for workers in [0, 1, 3] {
            let bytes = fixture();
            let pool = Pool::start(workers, 1, move |request: &mut Request| {
                request.run(&Movement(SliceMemory {
                    base: 0,
                    bytes: &bytes,
                }))
            })
            .unwrap();
            let owned = request(2);
            let pointer = owned.census.payload.as_ptr();
            let visitor_bytes = owned.visitor.allocated_bytes();
            let mut pending = Some(owned);
            // SAFETY: all sources and outputs are owned until collection.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            assert_eq!(
                completed.report.unwrap().census.result,
                Outcome::Rows { len: 2 }
            );
            assert_eq!(completed.request.census.payload.as_ptr(), pointer);
            assert_eq!(completed.request.visitor.allocated_bytes(), visitor_bytes);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }
}
