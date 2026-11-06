//! Owned States census and its callback-scoped StateFlags visitor.
//! Flags and active border trees use this worker's authenticated census;
//! assembly later consumes their already acquired timing and owned rows.
//! A [`StateStore`] carries the rows no hooked writer marked
//! ([`crate::territory_changes`]).

use crate::{acquisition_gather, extended::state_flags, sampling};
use engine_native::{Memory, cstate, walk::Outcome};
use engine_railways::crailway_manager::fresh_inputs;

mod occupation;
mod state_store;
pub(crate) use occupation::Store as OccupationStore;
pub(crate) use state_store::StateStore;

pub(crate) struct Request {
    pub census: acquisition_gather::Request<Vec<cstate::Row>>,
    pub names: Names,
    pub visitor: state_flags::Captured,
    pub inputs: Inputs,
    /// The held rows, lent by the callback; absent reads every state.
    pub store: Option<Box<StateStore>>,
    pub countries: Option<std::sync::Arc<engine_native::country_memory::Cache>>,
    pub occupation_store: Option<Box<OccupationStore>>,
}

/// State supplements acquired from the existing visitor; occupation receiver identities
/// survive only under their writer certificates.
pub(crate) struct Inputs {
    rows: Vec<fresh_inputs::StateFractionRow>,
    outcome: Outcome,
    required: usize,
    pub(crate) occupation: occupation::Occupation,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            outcome: Outcome::Unavailable {
                reason: "clock unavailable",
            },
            required: 0,
            occupation: Default::default(),
        }
    }
}

impl Inputs {
    pub(crate) fn with_capacity(capacity: usize) -> std::io::Result<Self> {
        Ok(Self {
            rows: engine_allocation::default_vec(capacity)?,
            ..Self::default()
        })
    }

    pub(crate) fn reset(&mut self) {
        self.occupation.reset();
        self.outcome = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        self.required = 0;
    }

    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.occupation.fail(reason);
        self.outcome = Outcome::Unavailable { reason };
    }

    pub(crate) fn required(&self) -> usize {
        self.required
    }

    pub(crate) fn retain_required(&mut self, required: usize) {
        self.required = self.required.max(required);
    }

    pub(crate) fn grow(&mut self, capacity: usize) {
        crate::table_capture::grow_vec(&mut self.rows, capacity);
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<fresh_inputs::StateFractionRow>()
            + self.occupation.bytes()
    }

    fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        visitor: &state_flags::Captured,
        growing: bool,
    ) {
        self.acquire_carried(memory, base, visitor, growing, |_| None);
    }

    /// [`Self::acquire`] where `carried` supplies, by source index, the
    /// fraction of a state the caller proves current.
    fn acquire_carried(
        &mut self,
        memory: &impl Memory,
        base: usize,
        visitor: &state_flags::Captured,
        growing: bool,
        carried: impl FnMut(usize) -> Option<Result<i64, &'static str>>,
    ) {
        let (sources, source, required) = visitor.completed_sources();
        self.required = self.required.max(required);
        if let Outcome::Unavailable { reason } = source {
            self.fail(reason);
            return;
        }
        let capacity = sources.len().max(self.rows.len());
        if growing && engine_allocation::resize_default(&mut self.rows, capacity).is_err() {
            self.fail("state fraction buffer allocation failed");
            return;
        }
        let outcome =
            fresh_inputs::state_fractions_carried(memory, base, sources, &mut self.rows, carried);
        self.outcome = match outcome {
            Outcome::Unavailable { .. } => outcome,
            _ => {
                let len = outcome.len();
                let omitted = self.required.saturating_sub(len);
                match u32::try_from(omitted) {
                    Ok(0) => Outcome::Rows { len },
                    Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
                    Err(_) => Outcome::Unavailable {
                        reason: "state fraction demand overflow",
                    },
                }
            }
        };
    }

    pub(crate) fn fraction(&self, state: u32) -> Option<i64> {
        let rows = self.rows.get(..self.outcome.len())?;
        rows.binary_search_by_key(&state, |row| row.state)
            .ok()
            .and_then(|index| rows[index].fraction.ok())
    }

    pub(crate) fn observe(&self, work: &mut InputWork) -> std::io::Result<()> {
        work.observe(
            self.outcome,
            self.required,
            self.rows.len(),
            self.rows[..self.outcome.len()]
                .iter()
                .map(|row| row.fraction.map(|_| true)),
        )
    }
}

/// Spool-thread totals preserve the exact static native refusal reasons.
#[derive(Default)]
pub(crate) struct InputWork {
    samples: u64,
    captured: u64,
    unqueried: u64,
    refused: u64,
    required: usize,
    capacity: usize,
    omissions: u64,
    reasons: Vec<(&'static str, u64)>,
}

impl InputWork {
    pub(crate) fn observe(
        &mut self,
        outcome: Outcome,
        required: usize,
        capacity: usize,
        results: impl Iterator<Item = Result<bool, &'static str>>,
    ) -> std::io::Result<()> {
        self.samples = self.samples.saturating_add(1);
        self.required = self.required.max(required);
        self.capacity = self.capacity.max(capacity);
        if let Outcome::Truncated { unread_rows, .. } = outcome {
            self.omissions = self.omissions.saturating_add(u64::from(unread_rows));
        }
        if let Outcome::Unavailable { reason } = outcome {
            self.refusal(reason)?;
        }
        for result in results {
            match result {
                Ok(true) => self.captured = self.captured.saturating_add(1),
                Ok(false) => self.unqueried = self.unqueried.saturating_add(1),
                Err(reason) => self.refusal(reason)?,
            }
        }
        Ok(())
    }

    fn refusal(&mut self, reason: &'static str) -> std::io::Result<()> {
        let index = match self.reasons.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                engine_allocation::reserve(&mut self.reasons, 1)?;
                self.reasons.insert(index, (reason, 0));
                index
            }
        };
        self.reasons[index].1 = self.reasons[index].1.saturating_add(1);
        self.refused = self.refused.saturating_add(1);
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "samples": self.samples,
            "captured": self.captured,
            "unqueried": self.unqueried,
            "refused": self.refused,
            "required": self.required,
            "capacity": self.capacity,
            "omissions": self.omissions,
            "refusals": crate::status::map(self.reasons.iter().map(|&(reason, count)| (reason, count))),
        })
    }
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
    pub occupation_ticks: i64,
}
impl Report {
    pub(crate) fn failed(reason: &'static str) -> Self {
        Self {
            census: acquisition_gather::Report {
                result: Outcome::Unavailable { reason },
            },
            ticks: 0,
            occupation_ticks: 0,
        }
    }
}
impl Request {
    #[cfg(test)]
    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory, false)
    }

    pub(crate) fn run_growing(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory, true)
    }

    fn run_with(&mut self, memory: &impl Memory, growing: bool) -> Report {
        let _reads = crate::read_accounting::table(replay_data::rows::TableId::States);
        let Self {
            census,
            names,
            visitor,
            inputs,
            store,
            countries,
            occupation_store,
        } = self;
        let (base, game_state) = (census.base, census.game_state);
        let begin = sampling::qpc();
        if let Some(store) = store.as_deref_mut() {
            store.begin(memory, base, game_state);
        }
        let report = census.run(|rows| {
            if let Some(store) = store.as_deref_mut()
                && let Some((outcome, walk)) = store.carry(
                    memory,
                    base,
                    game_state,
                    growing,
                    rows,
                    &mut names.bytes,
                    visitor,
                    inputs,
                )
            {
                names.len = walk.bytes_len;
                names.required = walk.required_bytes;
                store.finish(false, outcome.len());
                return walk.outcome;
            }
            let outcome = if growing {
                cstate::walk_with_growing(memory, base, game_state, rows, |source| {
                    visitor.visit_growing(source)
                })
            } else {
                cstate::walk_with(memory, base, game_state, rows, |source| {
                    visitor.visit(source)
                })
            };
            visitor.finish(outcome);
            cstate::read_continents(memory, base, game_state, outcome, rows);
            let outcome =
                cstate::read_movement_infrastructure(memory, base, game_state, outcome, rows);
            let walk = cstate::read_names_carried_scripted(
                memory,
                game_state,
                outcome,
                rows,
                engine_native::walk::Buffer::for_worker(&mut names.bytes, growing),
                |_| None,
                |id| {
                    store
                        .as_deref()
                        .map_or((false, false), |store| store.scripted(id))
                },
            );
            names.len = walk.bytes_len;
            names.required = walk.required_bytes;
            inputs.acquire(memory, base, visitor, growing);
            if let Some(store) = store.as_deref_mut() {
                store.hold(rows, &names.bytes, walk, visitor, inputs);
                store.finish(true, walk.outcome.len());
            }
            walk.outcome
        });
        match store.as_deref_mut() {
            Some(store) => store.infrastructure(
                memory,
                base,
                game_state,
                report.result,
                &mut census.payload,
            ),
            None => cstate::infrastructure_map::snapshot(
                memory,
                base,
                game_state,
                report.result,
                &mut census.payload,
            ),
        }
        let ticks = sampling::qpc() - begin;
        let occupation_begin = sampling::qpc();
        let (sources, source_outcome, _) = visitor.completed_sources();
        {
            let _reads = crate::read_accounting::table(replay_data::rows::TableId::StateResistance);
            if let Some(store) = occupation_store.as_deref_mut() {
                if let Some(countries) = countries.as_deref() {
                    let cached = countries.view(memory);
                    store.acquire(
                        &cached,
                        base,
                        game_state,
                        sources,
                        source_outcome,
                        Some(countries),
                    );
                } else {
                    store.acquire(memory, base, game_state, sources, source_outcome, None);
                }
                inputs.occupation.publish(store);
            } else {
                inputs.occupation.reset();
            }
        }
        let occupation_ticks = sampling::qpc() - occupation_begin;
        visitor.acquire_owned(memory, base, game_state, growing);
        Report {
            census: report,
            ticks,
            occupation_ticks,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::acquisition::Pool;
    use engine_native::{cbuilding_database, cbuilding_template, cgame_state, memory::SliceMemory};
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
                (cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
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
                    pointer + field + engine_native::msvc_string::CAPACITY,
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
    // The per-level define, fixed5.
    const PER_LEVEL: i64 = -10_000;
    // A disabled template lookup keeps every healthy level at zero, so the term
    // is maximum * per level. Nonzero, so a denied pass cannot be mistaken for it.
    const MOVEMENT_INFRASTRUCTURE: i64 = MAXIMUM as i64 * PER_LEVEL;

    /// The movement globals as (address, eight little-endian bytes).
    fn movement_globals() -> [(usize, [u8; 8]); 5] {
        [
            (
                cbuilding_database::SINGLETON_RVA.0,
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
            (0x033e_5080, PER_LEVEL.to_le_bytes()),
        ]
    }

    /// The rest of the shared infrastructure operands a held capture
    /// compares every hour ([`cstate::globals`]): the definition's type,
    /// fallback maximum and ordinal, the template array and the fallback
    /// define. None changes a fresh read of the fixture.
    fn carry_globals() -> [(usize, [u8; 16]); 5] {
        [
            (DEFINITION + 0x8, [0x2f; 16]),
            (DEFINITION + 0x2b8, [0; 16]),
            (DEFINITION + 0x2e0, [3; 16]),
            (DATABASE + 0x40, [0; 16]),
            (0x033e_5274, [0; 16]),
        ]
    }

    /// The movement pass chases module globals at RVAs far above any dense
    /// census fixture, so this overlays them on the fixture memory a test
    /// already uses. Wrapped by tracing memories, so its reads are observed.
    pub(crate) struct Movement<M>(pub(crate) M);

    fn overlay(at: usize, bytes: &[u8], address: usize, destination: &mut [u8]) -> bool {
        let Some(start) = address.checked_sub(at) else {
            return false;
        };
        let Some(end) = start
            .checked_add(destination.len())
            .filter(|end| *end <= bytes.len())
        else {
            return false;
        };
        destination.copy_from_slice(&bytes[start..end]);
        true
    }

    impl<M: Memory> Memory for Movement<M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (at, bytes) in movement_globals() {
                if overlay(at, &bytes, address, destination) {
                    return true;
                }
            }
            for (at, bytes) in carry_globals() {
                if overlay(at, &bytes, address, destination) {
                    return true;
                }
            }
            self.0.copy(address, destination)
        }
    }

    fn request(capacity: usize) -> Request {
        let mut flags = state_flags::Capacities::default();
        flags.raise_for_states(2);
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
            visitor: state_flags::Captured::with_capacities(&flags),
            inputs: Inputs::default(),
            store: None,
            countries: None,
            occupation_store: None,
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
        cstate::read_continents(memory, 0, STATE, outcome, &mut request.census.payload);
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
        cstate::infrastructure_map::snapshot(memory, 0, STATE, names.outcome, &mut request.census.payload);
        request.visitor.acquire_owned(memory, 0, STATE, true);
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
    fn growing_states_fills_the_first_census_without_repeating_its_root() {
        let bytes = fixture();
        assert_eq!(
            request(0).run(&Traced::new(&bytes)).census.result,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        let memory = Traced::new(&bytes);
        let mut request = request(0);
        request.visitor = state_flags::Captured::default();
        let report = request.run_growing(&memory);
        assert_eq!(report.census.result, Outcome::Rows { len: 2 });
        assert_eq!(request.census.payload[1].id, 3);
        assert_eq!(request.inputs.outcome, Outcome::Rows { len: 2 });
        // Census, continent supplement, movement and names each own their
        // original roster observation; growth adds no second census attempt.
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == STATE + cgame_state::STATES)
                .count(),
            4
        );
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
            let (at, len) = healthy.calls.borrow()[fault];
            // A state's category is an independent capture: its fault refuses
            // only that state's category.
            if len == 8 && [FIRST, LAST].contains(&at.wrapping_sub(cstate::CATEGORY)) {
                assert_eq!(report.census.result, Outcome::Rows { len: 2 });
                assert!(
                    actual
                        .census
                        .payload
                        .iter()
                        .any(|row| row.category.is_none())
                );
            } else if healthy.calls.borrow()[fault] == (0x033d_d4b8, 8) {
                // The continents database singleton is optional.
                assert_eq!(report.census.result, Outcome::Rows { len: 2 });
                assert_eq!(actual.visitor.outcomes(), [Outcome::Rows { len: 2 }]);
                assert!(
                    actual
                        .census
                        .payload
                        .iter()
                        .all(|row| row.continent.is_none())
                );
            } else {
                // A census fault fails the census; a flag fault fails only flags.
                assert!(
                    matches!(report.census.result, Outcome::Unavailable { .. })
                        || matches!(actual.visitor.outcomes(), [Outcome::Unavailable { .. }]),
                    "fault {fault}"
                );
            }
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

    #[test]
    fn fraction_control_failure_preserves_existing_states_names_and_flags() {
        let bytes = fixture();
        let mut healthy = request(2);
        healthy.inputs = Inputs::with_capacity(2).unwrap();
        let observed = Traced::new(&bytes);
        healthy.run(&observed);
        let fault = observed
            .calls
            .borrow()
            .iter()
            .rposition(|(address, _)| *address == cbuilding_database::SINGLETON_RVA.0)
            .unwrap();
        let mut memory = Traced::new(&bytes);
        memory.fault = Some(fault);
        let mut captured = request(2);
        captured.inputs = Inputs::with_capacity(2).unwrap();
        let report = captured.run(&memory);
        assert_eq!(report.census.result, Outcome::Rows { len: 2 });
        assert_eq!(captured.census.payload, healthy.census.payload);
        assert_eq!(captured.names.len, healthy.names.len);
        assert_eq!(captured.visitor.outcomes(), [Outcome::Rows { len: 2 }]);
        assert_eq!(captured.inputs.required(), 2);
        assert_eq!(captured.inputs.fraction(1), None);
        assert_eq!(captured.inputs.fraction(3), None);
        assert_eq!(healthy.inputs.fraction(1), Some(0));
        assert_eq!(healthy.inputs.fraction(3), Some(0));
        let mut work = InputWork::default();
        captured.inputs.observe(&mut work).unwrap();
        assert_eq!(work.refused, 2);
        assert_eq!(work.captured, 0);
        assert_eq!(work.required, 2);
    }

    #[test]
    fn later_old_field_failure_keeps_fraction_prefix_and_complete_core_demand() {
        let bytes = fixture();
        let observed = Traced::new(&bytes);
        request(1).run(&observed);
        let fault = observed
            .calls
            .borrow()
            .iter()
            .position(|(address, _)| *address == cbuilding_database::SINGLETON_RVA.0)
            .unwrap();
        let mut memory = Traced::new(&bytes);
        memory.fault = Some(fault);
        let mut captured = request(1);
        captured.inputs = Inputs::with_capacity(1).unwrap();
        let report = captured.run(&memory);
        assert!(matches!(report.census.result, Outcome::Unavailable { .. }));
        assert_eq!(captured.inputs.required(), 2);
        assert_eq!(
            captured.inputs.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(captured.inputs.fraction(1), Some(0));
        assert_eq!(captured.inputs.fraction(3), None);
        captured.inputs.fail("worker failed after census");
        assert_eq!(captured.inputs.required(), 2);
        assert_eq!(captured.inputs.fraction(1), None);
        assert!(captured.visitor.completed_sources().0.is_empty());
    }

    #[test]
    fn mandatory_state_failure_never_admits_a_visited_fraction_source() {
        let bytes = fixture();
        let observed = Traced::new(&bytes);
        request(2).run(&observed);
        let fault = observed
            .calls
            .borrow()
            .iter()
            .position(|(address, _)| *address == FIRST + cstate::MANPOWER_AVAILABLE)
            .unwrap();
        let mut memory = Traced::new(&bytes);
        memory.fault = Some(fault);
        let mut captured = request(2);
        captured.inputs = Inputs::with_capacity(2).unwrap();
        let report = captured.run(&memory);
        assert!(matches!(report.census.result, Outcome::Unavailable { .. }));
        assert!(matches!(
            captured.inputs.outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.inputs.fraction(1), None);
        assert!(captured.visitor.completed_sources().0.is_empty());
    }

    #[test]
    fn worker_failure_before_supplement_keeps_visited_demand_for_core_growth() {
        let mut frame = sampling::Captured::new();
        frame.states = vec![cstate::Row::default(); 1];
        frame.state_inputs = Inputs::with_capacity(1).unwrap();
        let mut request = frame.take_states_request(0, STATE);
        request.visitor.visit(cstate::Source {
            pointer: FIRST,
            id: 1,
        });
        request.visitor.finish(Outcome::Truncated {
            len: 1,
            unread_rows: 4096,
        });
        assert_eq!(request.inputs.required(), 0);
        frame.restore_states_request(request, Report::default(), Some("states worker panicked"));
        assert_eq!(
            frame.state_inputs.occupation.outcome(),
            Outcome::Unavailable {
                reason: "states worker panicked",
            }
        );
        assert_eq!(frame.state_inputs.required(), 4097);
        assert_eq!(frame.state_inputs.fraction(1), None);
        let visitor = frame.extended.take_state_visitor();
        assert!(visitor.completed_sources().0.is_empty());
        frame.extended.restore_state_visitor(visitor);
        let mut capacities = sampling::INITIAL_CAPACITIES;
        capacities[3] = 1;
        frame.grow(&mut capacities);
        assert!(capacities[3] >= 4097);
        assert_eq!(frame.states.len(), capacities[3]);
        assert_eq!(frame.state_inputs.rows.len(), capacities[3]);
        let mut work = InputWork::default();
        frame.state_inputs.observe(&mut work).unwrap();
        assert_eq!(work.reasons, [("states worker panicked", 1)]);
        assert_eq!(work.required, 4097);
    }

    mod carry;
}
