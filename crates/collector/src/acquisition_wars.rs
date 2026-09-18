//! The owned war census and its independent, separately gated matrix census.
//! Every native address is consumed inside the callback's acquisition lease.

use crate::{acquisition_gather, sampling};
use engine::{Memory, country_memory::Cache, cwar_relation, walk::Outcome};
use std::sync::Arc;

pub(crate) struct Request {
    pub countries: Arc<Cache>,
    pub table: acquisition_gather::Request<Vec<cwar_relation::Row>>,
    pub snapshot: cwar_relation::OwnedSnapshotScratch,
    // Retained with the owned request even when its runner unwinds.
    pub snapshot_sampled: bool,
    pub matrix: Vec<cwar_relation::Row>,
    pub matrix_needed: bool,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub table: acquisition_gather::Report,
    pub verification_failed: bool,
    pub matrix: Option<Outcome>,
    pub table_ticks: i64,
    pub matrix_ticks: i64,
}

impl Report {
    pub(crate) fn failed(reason: &'static str, matrix_needed: bool) -> Self {
        let unavailable = Outcome::Unavailable { reason };
        Self {
            table: acquisition_gather::Report {
                result: unavailable,
                second: unavailable,
                mismatches: 0,
            },
            matrix: matrix_needed.then_some(unavailable),
            ..Self::default()
        }
    }

    pub(crate) fn table_matched(self) -> bool {
        !self.verification_failed
            && sampling::full_match(self.table.result, self.table.second, self.table.mismatches)
    }

    /// Eligibility for the existing callback window. Full row-set comparison
    /// remains in spooling after acquisition, where native order is irrelevant.
    pub(crate) fn matrix_eligible(self) -> bool {
        !self.verification_failed
            && matches!(
                (self.table.result, self.matrix),
                (Outcome::Rows { .. }, Some(Outcome::Rows { .. }))
            )
            && self.table.mismatches == 0
    }
}

impl Request {
    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        let Self {
            countries,
            table,
            snapshot,
            snapshot_sampled,
            matrix,
            matrix_needed,
        } = self;
        let cached = countries.view(memory);
        let needed = table.needed;
        let base = table.base;
        let game_state = table.game_state;
        let mut verification_failed = false;
        let begin = sampling::qpc();
        *snapshot_sampled = true;
        let mut table = table.buffers.rows(needed, |rows, second| {
            if second {
                cwar_relation::walk(&cached, base, game_state, rows)
            } else {
                // The verifier must see fresh underlying memory, never the
                // acquisition view or bytes retained by the ownership snapshot.
                let read = cwar_relation::walk_verified(
                    &cached,
                    memory,
                    base,
                    game_state,
                    rows,
                    snapshot,
                    needed || *matrix_needed,
                );
                verification_failed = read.verification_failed;
                read.outcome
            }
        });
        if needed {
            table.mismatches += usize::from(verification_failed);
        }
        let table_ticks = sampling::qpc() - begin;
        // Even a failed owned pass cannot replace or suppress the independent
        // matrix walk while that census's own window remains open.
        let (matrix, matrix_ticks) = if *matrix_needed {
            let begin = sampling::qpc();
            let outcome = cwar_relation::walk_matrix(&cached, base, game_state, matrix);
            (Some(outcome), sampling::qpc() - begin)
        } else {
            (None, 0)
        };
        Report {
            table,
            verification_failed,
            matrix,
            table_ticks,
            matrix_ticks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acquisition::{Failure, Pool, SubmitError},
        acquisition_gather::Buffers,
        sampling::wars_test_kit as kit,
    };
    use std::{cell::RefCell, ops::Range};

    #[derive(Clone, Copy)]
    enum Action {
        Change,
        Fail,
        Panic,
    }

    struct ScriptMemory<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<(usize, usize)>>,
        injection: Option<(usize, Action)>,
        blocked: Range<usize>,
    }

    impl<'a> ScriptMemory<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                bytes,
                reads: RefCell::new(Vec::new()),
                injection: None,
                blocked: 0..0,
            }
        }
    }

    impl Memory for ScriptMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let index = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            if address < self.blocked.end && address + output.len() > self.blocked.start {
                return false;
            }
            let copied = kit::memory(self.bytes).copy(address, output);
            if let Some((at, action)) = self.injection
                && at == index
            {
                match action {
                    Action::Change => output[0] ^= 8,
                    Action::Fail => return false,
                    Action::Panic => panic!("synthetic wars source panic"),
                }
            }
            copied
        }
    }

    fn request(bytes: &[u8], capacity: usize, needed: bool, matrix_needed: bool) -> Request {
        let memory = kit::memory(bytes);
        let mut countries = Cache::with_capacity(1024);
        assert_eq!(
            countries
                .prepare(&memory, kit::BASE, kit::GAME_STATE)
                .seed_status(),
            engine::country_memory::SeedStatus::Ready
        );
        let mut snapshot = cwar_relation::OwnedSnapshotScratch::with_capacity(5);
        // Discover demand and grow outside acquisition, as the spooler does.
        let _ = cwar_relation::walk_verified(
            &memory,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            &mut [cwar_relation::Row::default(); 4],
            &mut snapshot,
            false,
        );
        snapshot.grow(5);
        let sentinel = cwar_relation::Row {
            cancel: 0xa5,
            ..Default::default()
        };
        Request {
            countries: Arc::new(countries),
            table: acquisition_gather::Request {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                needed,
                buffers: Buffers {
                    first: vec![sentinel; capacity],
                    second: vec![sentinel; capacity],
                },
            },
            snapshot,
            snapshot_sampled: false,
            matrix: vec![sentinel; capacity],
            matrix_needed,
        }
    }

    fn row_pointers(request: &Request) -> [usize; 3] {
        [
            request.table.buffers.first.as_ptr() as usize,
            request.table.buffers.second.as_ptr() as usize,
            request.matrix.as_ptr() as usize,
        ]
    }

    #[test]
    fn independent_windows_and_capacities_preserve_rows_and_storage() {
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for capacity in [0, 1, 2, 4] {
            let expected = kit::direct(&memory, capacity);
            let mut reference = request(&bytes, capacity, true, true);
            let _ = cwar_relation::walk_verified(
                &memory,
                &memory,
                kit::BASE,
                kit::GAME_STATE,
                &mut reference.table.buffers.first,
                &mut reference.snapshot,
                true,
            );
            let verified_stats = reference.snapshot.stats();
            assert!(verified_stats.hits > 0);
            assert!(verified_stats.verification_headers > 0);
            assert!(verified_stats.verification_words > 0);
            for needed in [false, true] {
                for matrix_needed in [false, true] {
                    let mut request = request(&bytes, capacity, needed, matrix_needed);
                    let pointers = row_pointers(&request);
                    let snapshot_capacity = request.snapshot.capacity();
                    let first_before = request.table.buffers.first.clone();
                    let second_before = request.table.buffers.second.clone();
                    let matrix_before = request.matrix.clone();
                    let report = request.run(&memory);
                    let len = report.table.result.len();
                    assert_eq!(report.table.result, expected.owned_outcome);
                    assert_eq!(&request.table.buffers.first[..len], &expected.owned[..len]);
                    assert_eq!(&request.table.buffers.first[len..], &first_before[len..]);
                    assert_eq!(report.table.mismatches, 0);
                    assert_eq!(report.table_matched(), needed && capacity >= 2);
                    assert_eq!(report.matrix_eligible(), matrix_needed && capacity >= 2);
                    if needed {
                        assert_eq!(report.table.second, expected.owned_outcome);
                        assert_eq!(request.table.buffers.second, request.table.buffers.first);
                    } else {
                        assert!(matches!(report.table.second, Outcome::Unavailable { .. }));
                        assert_eq!(request.table.buffers.second, second_before);
                    }
                    if matrix_needed {
                        let len = expected.matrix_outcome.len();
                        assert_eq!(report.matrix, Some(expected.matrix_outcome));
                        assert_eq!(&request.matrix[..len], &expected.matrix[..len]);
                        assert_eq!(&request.matrix[len..], &matrix_before[len..]);
                    } else {
                        assert_eq!(report.matrix, None);
                        assert_eq!(report.matrix_ticks, 0);
                        assert_eq!(request.matrix, matrix_before);
                    }
                    let stats = request.snapshot.stats();
                    assert_eq!(stats.hits, verified_stats.hits);
                    assert_eq!(
                        stats.verification_headers,
                        verified_stats.verification_headers * usize::from(needed || matrix_needed)
                    );
                    assert_eq!(
                        stats.verification_words,
                        verified_stats.verification_words * usize::from(needed || matrix_needed)
                    );
                    assert!(request.snapshot_sampled);
                    assert_eq!(request.snapshot.capacity(), snapshot_capacity);
                    assert_eq!(row_pointers(&request), pointers);
                }
            }
        }
    }

    #[test]
    fn fresh_snapshot_header_and_word_regressions_withhold_both_credits() {
        let bytes = kit::fixture();
        let baseline = ScriptMemory::new(&bytes);
        let _ = request(&bytes, 4, false, false).run(&baseline);
        // Verification follows the complete first pass. Its first observations
        // are the reused ownership header and its first ordered pointer word.
        let verification = baseline.reads.borrow().len();
        for (offset, address) in [
            (0, kit::D0 + engine::cdiplomacy_status::OWNED_RELATIONS),
            (1, kit::O0),
        ] {
            for action in [Action::Change, Action::Fail] {
                for (needed, matrix_needed) in [(true, false), (false, true), (true, true)] {
                    let mut memory = ScriptMemory::new(&bytes);
                    memory.injection = Some((verification + offset, action));
                    let mut request = request(&bytes, 4, needed, matrix_needed);
                    let report = request.run(&memory);
                    assert_eq!(memory.reads.borrow()[verification + offset].0, address);
                    assert!(report.verification_failed);
                    assert!(request.snapshot.stats().invalidated);
                    assert!(matches!(report.table.result, Outcome::Unavailable { .. }));
                    assert_eq!(report.table.mismatches, if needed { 2 } else { 0 });
                    assert!(!report.table_matched());
                    assert!(!report.matrix_eligible());
                    if needed {
                        // A failed snapshot neither suppresses nor blesses the
                        // independent second table pass, which still completes.
                        assert_eq!(report.table.second, Outcome::Rows { len: 2 });
                    }
                    assert_eq!(
                        report.matrix,
                        matrix_needed.then_some(Outcome::Rows { len: 2 })
                    );
                }
            }
        }
    }

    #[test]
    fn second_table_source_change_is_compared_before_matrix_eligibility() {
        let bytes = kit::fixture();
        let baseline = ScriptMemory::new(&bytes);
        let _ = request(&bytes, 4, true, true).run(&baseline);
        let second_casualties = baseline
            .reads
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (address, _))| *address == kit::WAR1 + cwar_relation::FIRST_CASUALTIES)
            .nth(1)
            .unwrap()
            .0;
        let mut memory = ScriptMemory::new(&bytes);
        memory.injection = Some((second_casualties, Action::Change));
        let mut request = request(&bytes, 4, true, true);
        let report = request.run(&memory);
        assert_eq!(report.table.result, Outcome::Rows { len: 2 });
        assert_eq!(report.table.second, Outcome::Rows { len: 2 });
        assert_eq!(report.table.mismatches, 1);
        assert!(!report.verification_failed);
        assert!(!report.table_matched());
        assert!(!report.matrix_eligible());
        assert_eq!(report.matrix, Some(Outcome::Rows { len: 2 }));
        assert_eq!(&request.table.buffers.first[..2], &request.matrix[..2]);
    }

    #[test]
    fn complete_changed_matrix_keeps_rows_and_sampler_eligibility() {
        let bytes = kit::fixture();
        for needed in [false, true] {
            let baseline = ScriptMemory::new(&bytes);
            let _ = request(&bytes, 4, needed, true).run(&baseline);
            let matrix_casualties = baseline
                .reads
                .borrow()
                .iter()
                .rposition(|&(address, _)| address == kit::WAR1 + cwar_relation::FIRST_CASUALTIES)
                .unwrap();
            let mut memory = ScriptMemory::new(&bytes);
            memory.injection = Some((matrix_casualties, Action::Change));
            let mut request = request(&bytes, 4, needed, true);
            let matrix_pointer = request.matrix.as_ptr();
            let report = request.run(&memory);
            assert_eq!(report.table.result, Outcome::Rows { len: 2 });
            assert_eq!(report.matrix, Some(Outcome::Rows { len: 2 }));
            assert_eq!(report.table.mismatches, 0);
            assert!(!report.verification_failed);
            assert_eq!(report.table_matched(), needed);
            assert!(report.matrix_eligible());
            // Eligibility does not compare census rows. Preserve the complete
            // differing matrix for the existing off-thread spooler check.
            let mut changed = request.table.buffers.first[0];
            changed.first_casualties ^= 8;
            assert_eq!(request.matrix[0], changed);
            assert_eq!(request.matrix[1], request.table.buffers.first[1]);
            let mut frame = sampling::Captured::new();
            let report = frame.restore_wars_request(request, report, None);
            assert!(report.matrix_eligible());
            assert_eq!(frame.war_matrix_result, Some(Outcome::Rows { len: 2 }));
            assert_eq!(frame.wars_matrix.as_ptr(), matrix_pointer);
            assert_eq!(frame.wars_matrix[0], changed);
            assert_ne!(frame.wars_matrix[0], frame.wars[0]);
        }
    }

    #[test]
    fn matrix_and_owned_failures_keep_their_independent_censuses() {
        let bytes = kit::fixture();
        for (blocked, owned_ok, matrix_ok) in [
            (kit::M0 + 8, true, false),
            (
                kit::D0 + engine::cdiplomacy_status::OWNED_RELATIONS,
                false,
                true,
            ),
            (kit::WAR1 + cwar_relation::FIRST_CASUALTIES, false, false),
        ] {
            let mut memory = ScriptMemory::new(&bytes);
            memory.blocked = blocked..blocked + 1;
            let mut request = request(&bytes, 4, true, true);
            let report = request.run(&memory);
            assert_eq!(
                matches!(report.table.result, Outcome::Rows { .. }),
                owned_ok
            );
            assert_eq!(
                matches!(report.matrix, Some(Outcome::Rows { .. })),
                matrix_ok
            );
            assert_eq!(report.table_matched(), owned_ok);
            assert!(!report.matrix_eligible());
        }
        let mut memory = ScriptMemory::new(&bytes);
        memory.blocked = kit::M0 + 8..kit::M0 + 9;
        let report = request(&bytes, 4, true, false).run(&memory);
        assert!(report.table_matched());
        assert_eq!(report.matrix, None);
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == kit::M0 + 8)
        );
    }

    #[test]
    fn successive_callbacks_discard_snapshot_keys_and_refresh_country_lease() {
        let bytes = kit::fixture();
        let mut request = request(&bytes, 4, true, true);
        assert!(request.run(&kit::memory(&bytes)).table_matched());
        let snapshot_capacity = request.snapshot.capacity();
        let mut changed = bytes.clone();
        changed[kit::WAR1 + cwar_relation::FIRST_CASUALTIES] ^= 8;
        changed[kit::O0..kit::O0 + 8].copy_from_slice(&(kit::WAR2 as u64).to_le_bytes());
        changed[kit::O0 + 16..kit::O0 + 24].copy_from_slice(&(kit::WAR1 as u64).to_le_bytes());
        let memory = kit::memory(&changed);
        let mut countries = Cache::with_capacity(1024);
        let _ = countries.prepare(&memory, kit::BASE, kit::GAME_STATE);
        request.countries = Arc::new(countries);
        let report = request.run(&memory);
        assert!(report.table_matched());
        assert!(report.matrix_eligible());
        let direct = kit::direct(&memory, 4);
        assert_eq!(&request.table.buffers.first[..2], &direct.owned[..2]);
        assert_eq!(&request.matrix[..2], &direct.matrix[..2]);
        assert_eq!(request.snapshot.capacity(), snapshot_capacity);
    }

    #[test]
    fn pool_source_panic_returns_all_storage_and_cancels_peer_without_credit() {
        let bytes = kit::fixture();
        let baseline = ScriptMemory::new(&bytes);
        let _ = request(&bytes, 4, false, false).run(&baseline);
        let verification = baseline.reads.borrow().len();
        let mut first = Some(request(&bytes, 4, true, true));
        let mut second = Some(request(&bytes, 4, true, true));
        let first_pointers = row_pointers(first.as_ref().unwrap());
        let second_pointers = row_pointers(second.as_ref().unwrap());
        let snapshot_capacity = first.as_ref().unwrap().snapshot.capacity();
        let country = Arc::clone(&first.as_ref().unwrap().countries);
        let pool = Pool::start(0, 2, move |request: &mut Request| {
            let mut memory = ScriptMemory::new(&bytes);
            memory.injection = Some((verification, Action::Panic));
            request.run(&memory)
        })
        .unwrap();
        // All source storage is Rust-owned for the lifetime of these tickets.
        let first_ticket = unsafe { pool.submit(&mut first) }.unwrap();
        let second_ticket = unsafe { pool.submit(&mut second) }.unwrap();
        let first = pool.finish(first_ticket).unwrap();
        let second = pool.finish(second_ticket).unwrap();
        assert!(matches!(first.report, Err(Failure::Panicked)));
        assert!(matches!(second.report, Err(Failure::Stopped)));
        assert_eq!(row_pointers(&first.request), first_pointers);
        assert_eq!(row_pointers(&second.request), second_pointers);
        assert_eq!(first.request.snapshot.capacity(), snapshot_capacity);
        assert!(first.request.snapshot_sampled);
        assert!(!second.request.snapshot_sampled);
        assert_eq!(first.request.table.buffers.first[0].first_country, 1);
        for report in [first.report, second.report] {
            let report = report.unwrap_or_default();
            assert!(!report.table_matched());
            assert!(!report.matrix_eligible());
        }
        drop(first);
        drop(second);
        assert_eq!(Arc::strong_count(&country), 1);
        assert_eq!(pool.calibration().uncollected, 0);
    }

    #[test]
    fn rejected_and_completed_jobs_keep_their_owned_buffers() {
        for workers in [0, 2] {
            let bytes = kit::fixture();
            let mut first = Some(request(&bytes, 4, true, true));
            let mut rejected = Some(request(&bytes, 4, false, false));
            let first_pointers = row_pointers(first.as_ref().unwrap());
            let rejected_pointers = row_pointers(rejected.as_ref().unwrap());
            let source = bytes.clone();
            let pool = Pool::start(workers, 1, move |request: &mut Request| {
                request.run(&kit::memory(&source))
            })
            .unwrap();
            let ticket = unsafe { pool.submit(&mut first) }.unwrap();
            assert_eq!(
                unsafe { pool.submit(&mut rejected) },
                Err(SubmitError::Full)
            );
            assert_eq!(row_pointers(rejected.as_ref().unwrap()), rejected_pointers);
            let done = pool.finish(ticket).unwrap();
            assert!(done.report.unwrap().table_matched());
            assert_eq!(row_pointers(&done.request), first_pointers);
            let report = rejected.as_mut().unwrap().run(&kit::memory(&bytes));
            assert_eq!(report.table.result, Outcome::Rows { len: 2 });
            assert_eq!(report.matrix, None);
            assert_eq!(row_pointers(rejected.as_ref().unwrap()), rejected_pointers);
            assert_eq!(pool.calibration().accepted, 1);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }
}
