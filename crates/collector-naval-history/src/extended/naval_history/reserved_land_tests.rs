use super::naval_test_kit as kit;
use super::*;
use std::cell::RefCell;

const ZERO: usize = 0x70000;
const ZERO_VECTOR: usize = 0x73000;
const ZERO_THEATRE: usize = 0x74000;
#[cfg(test)]
const ZERO_REGULAR: usize = 0x75000;
#[cfg(test)]
const OOB_GROUP: usize = 0x62000;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn vector(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}
pub fn fixture(count: u32) -> Vec<u8> {
    let mut bytes = kit::fixture();
    p64(&mut bytes, 0x11000, ZERO as u64);
    p64(
        &mut bytes,
        ZERO,
        engine_native::ccountry::VTABLE_RVA.0 as u64,
    );
    p32(&mut bytes, ZERO + engine_native::ccountry::ID, 0);
    vector(
        &mut bytes,
        ZERO + engine_native::ccountry::THEATRES,
        ZERO_VECTOR,
        count,
    );
    for ordinal in 0..count as usize {
        let theatre = ZERO_THEATRE + ordinal * ctheatre::SIZE;
        p64(&mut bytes, ZERO_VECTOR + ordinal * 8, theatre as u64);
        p64(&mut bytes, theatre, ctheatre::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            theatre + ctheatre::HANDLE,
            ((2 + ordinal as u64) << 32) | u64::from(ctheatre::REFERENCE_TYPE),
        );
        vector(&mut bytes, theatre + ctheatre::ORDERS_GROUP_DATA, 0, 0);
        vector(&mut bytes, theatre + ctheatre::ARMY_GROUP_DATA, 0, 0);
    }
    bytes
}

pub struct Observed<'a> {
    pub bytes: &'a [u8],
    pub fault: Option<usize>,
    pub calls: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Observed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, output.len()));
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        if self
            .fault
            .is_some_and(|fault| address <= fault && fault < end)
        {
            return false;
        }
        kit::memory(self.bytes).copy(address, output)
    }
}
impl Observed<'_> {
    #[cfg(test)]
    fn count(&self, address: usize) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|call| call.0 == address)
            .count()
    }
}

#[cfg(test)]
fn context() -> NavalCopyContext {
    NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    }
}
#[cfg(test)]
fn stage(memory: &Observed<'_>) -> (Captured, NavalJob) {
    let mut job = NavalJob::fixture();
    kit::stage_wings(memory, &mut job);
    assert!(copy_naval_refresh(memory, context(), &mut job).raw_ok);
    let native_calls = memory.calls.borrow().len();
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
    assert_eq!(
        memory.calls.borrow().len(),
        native_calls,
        "decode followed live memory"
    );
    (captured, job)
}
pub fn typed(memory: &Observed<'_>, land_capacity: usize) -> Captured {
    let mut capacity = Capacities::fixture();
    capacity.rows[4] = land_capacity;
    let mut captured = Captured::with_capacities(&capacity);
    captured.prepare_hour(0, true);
    captured.land.rows[0] = ctheatre::Row {
        id: (1u64 << 32) | ctheatre::REFERENCE_TYPE as u64,
        owner: 1,
        volunteers_theatre: 0,
        ordinal: 0,
    };
    captured.land.outcome = Outcome::Rows { len: 1 };
    let mut roots = vec![corders_group::Root::default(); 8];
    let outcome = corders_group::walk_roots(memory, kit::BASE, &mut roots);
    captured.set_order_sources(memory, kit::BASE, &roots, outcome);
    memory.calls.borrow_mut().clear();
    captured.capture_reserved_land(
        memory,
        kit::BASE,
        engine_native::ccountry::read_reserved_root(memory, kit::BASE, kit::GAME_STATE),
        None,
        None,
    );
    captured
}

#[test]
fn unknown_zero_theatre_requires_both_fresh_empty_rosters() {
    let bytes = fixture(1);
    let memory = Observed {
        bytes: &bytes,
        fault: None,
        calls: RefCell::new(Vec::new()),
    };
    let direct = typed(&memory, 2);
    assert_eq!(direct.reserved_land_count(), Some(1));
    assert_eq!(direct.land.outcome, Outcome::Rows { len: 2 });
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA), 1);
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ARMY_GROUP_DATA), 1);
    memory.calls.borrow_mut().clear();
    let (staged, job) = stage(&memory);
    assert_eq!(staged.reserved_land_count(), Some(1));
    assert_eq!(staged.land.outcome, direct.land.outcome);
    assert_eq!(staged.land.rows[..2], direct.land.rows[..2]);
    assert_eq!(job.reserved_raw_demand, Some((80, 7)));
    assert_eq!(job.log.spans.len() - job.positive_span_end, 7);
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA), 1);
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ARMY_GROUP_DATA), 1);
}

#[test]
fn unknown_nonempty_regular_or_army_roster_never_means_empty() {
    for offset in [ctheatre::ORDERS_GROUP_DATA, ctheatre::ARMY_GROUP_DATA] {
        let mut bytes = fixture(1);
        vector(&mut bytes, ZERO_THEATRE + offset, ZERO_REGULAR, 1);
        p64(&mut bytes, ZERO_REGULAR, OOB_GROUP as u64);
        let memory = Observed {
            bytes: &bytes,
            fault: None,
            calls: RefCell::new(Vec::new()),
        };
        let direct = typed(&memory, 2);
        assert_eq!(direct.reserved_land_count(), None);
        assert_eq!(direct.land.outcome, Outcome::Rows { len: 1 });
        let (staged, _) = stage(&memory);
        assert_eq!(staged.reserved_land_count(), None);
        assert_eq!(staged.land.outcome, direct.land.outcome);
        assert_eq!(
            memory.count(ZERO_REGULAR),
            0,
            "unknown roster entries were read"
        );
    }
}

#[test]
fn current_zero_roster_reuses_complete_oob_without_another_header_read() {
    let mut bytes = fixture(1);
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::THEATRE,
        ZERO_THEATRE as u64,
    );
    vector(
        &mut bytes,
        ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA,
        ZERO_REGULAR,
        1,
    );
    p64(&mut bytes, ZERO_REGULAR, OOB_GROUP as u64);
    let memory = Observed {
        bytes: &bytes,
        fault: None,
        calls: RefCell::new(Vec::new()),
    };
    let direct = typed(&memory, 2);
    assert_eq!(direct.reserved_land_count(), Some(1));
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA), 0);
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ARMY_GROUP_DATA), 0);
    memory.calls.borrow_mut().clear();
    let (staged, job) = stage(&memory);
    assert_eq!(staged.reserved_land_count(), Some(1));
    assert_eq!(job.reserved_raw_demand, Some((48, 5)));
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA), 1);
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ARMY_GROUP_DATA), 1);
    assert_eq!(staged.groups.rows[0].theatre_ordinal, Some(0));
}

#[test]
fn zero_roster_reuses_a_registry_authenticated_type53_root_with_id_word_zero() {
    let mut bytes = fixture(1);
    let handle = u64::from(corders_group::REFERENCE_TYPE);
    p64(&mut bytes, OOB_GROUP + corders_group::HANDLE, handle);
    p64(
        &mut bytes,
        0x61000 + engine_native_parts::reference_bucket::TYPE,
        handle,
    );
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::THEATRE,
        ZERO_THEATRE as u64,
    );
    vector(
        &mut bytes,
        ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA,
        ZERO_REGULAR,
        1,
    );
    p64(&mut bytes, ZERO_REGULAR, OOB_GROUP as u64);
    let memory = Observed {
        bytes: &bytes,
        fault: None,
        calls: RefCell::new(Vec::new()),
    };
    let direct = typed(&memory, 2);
    assert_eq!(direct.groups.outcome, Outcome::Rows { len: 1 });
    assert_eq!(direct.groups.rows[0].handle, handle);
    assert_eq!(direct.groups.rows[0].theatre_ordinal, Some(0));
    assert_eq!(direct.reserved_land_count(), Some(1));
    let (staged, job) = stage(&memory);
    assert_eq!(staged.groups.rows[0].handle, handle);
    assert_eq!(staged.reserved_land_count(), Some(1));
    assert_eq!(job.reserved_raw_demand, Some((48, 5)));
}

#[test]
fn failed_prior_roster_is_not_retried_and_keeps_oob_failure_precedence() {
    let mut bytes = fixture(1);
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::THEATRE,
        ZERO_THEATRE as u64,
    );
    let fault = ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA;
    let memory = Observed {
        bytes: &bytes,
        fault: Some(fault),
        calls: RefCell::new(Vec::new()),
    };
    let direct = typed(&memory, 2);
    assert!(matches!(direct.groups.outcome, Outcome::Unavailable { .. }));
    let (staged, job) = stage(&memory);
    assert_eq!(staged.land.outcome, Outcome::Rows { len: 1 });
    assert_eq!(staged.reserved_land_count(), None);
    assert!(job.reserved_first_failure.is_some());
    assert_eq!(memory.count(fault), 1);
}

#[test]
fn genuine_empty_country_zero_does_not_demand_unrelated_oob_success() {
    let bytes = fixture(0);
    let memory = Observed {
        bytes: &bytes,
        fault: Some(OOB_GROUP + corders_group::MEMBER_DATA),
        calls: RefCell::new(Vec::new()),
    };
    let direct = typed(&memory, 1);
    assert!(matches!(direct.groups.outcome, Outcome::Unavailable { .. }));
    assert_eq!(direct.reserved_land_count(), Some(0));
    let (staged, job) = stage(&memory);
    assert_eq!(staged.reserved_land_count(), Some(0));
    assert_eq!(staged.land.outcome, Outcome::Rows { len: 1 });
    assert_eq!(job.reserved_raw_demand, Some((16, 1)));
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ORDERS_GROUP_DATA), 0);
    assert_eq!(memory.count(ZERO_THEATRE + ctheatre::ARMY_GROUP_DATA), 0);
}

#[test]
fn optional_shortages_grow_in_the_pass_without_promoting_positive_faults() {
    let bytes = fixture(1);
    let memory = Observed {
        bytes: &bytes,
        fault: Some(OOB_GROUP + corders_group::MEMBER_DATA),
        calls: RefCell::new(Vec::new()),
    };
    let mut direct = typed(&memory, 1);
    assert_eq!(direct.land.outcome, Outcome::Rows { len: 1 });
    assert_eq!(direct.reserved_land_count(), None);
    assert_eq!(direct.reserved_land_required_rows, 1);
    let mut capacity = Capacities::fixture();
    capacity.rows[4] = 1;
    direct.grow(&mut capacity);
    assert!(capacity.rows[4] >= 2);
    assert!(direct.land.rows.len() >= 2);
    assert_eq!(direct.land.outcome, Outcome::Rows { len: 1 });
    let (_, baseline) = stage(&memory);
    let prefix_bytes = baseline.log.spans[..baseline.positive_span_end]
        .iter()
        .filter(|span| span.ok)
        .map(|span| span.start as usize + span.len as usize)
        .max()
        .unwrap();
    let mut job = NavalJob {
        log: RawLog::new(baseline.positive_span_end + 5, prefix_bytes + 48),
        ..Default::default()
    };
    kit::stage_wings(&memory, &mut job);
    assert!(copy_naval_refresh(&memory, context(), &mut job).raw_ok);
    assert!(job.log.fault.iter().any(|fault| *fault));
    assert!(!job.reserved_capacity_short);
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
    assert_eq!(captured.land.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.reserved_land_count(), None);
}

#[test]
fn optional_batch_never_promotes_incomplete_positive_theatres() {
    let bytes = fixture(1);
    let memory = Observed {
        bytes: &bytes,
        fault: None,
        calls: RefCell::new(Vec::new()),
    };
    for outcome in [
        Outcome::Unavailable {
            reason: "positive header failed",
        },
        Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        },
    ] {
        let mut captured = typed(&memory, 2);
        captured.land.outcome = outcome;
        captured.capture_reserved_land(
            &memory,
            kit::BASE,
            engine_native::ccountry::read_reserved_root(&memory, kit::BASE, kit::GAME_STATE),
            None,
            None,
        );
        assert_eq!(captured.land.outcome, outcome);
        assert_eq!(captured.reserved_land_count(), None);
        assert_eq!(captured.reserved_land_required_rows, 1);
        captured.reset();
        assert_eq!(captured.reserved_land_required_rows, 0);
    }
}

#[test]
fn ambiguous_prior_header_refuses_instead_of_selecting_or_retrying_an_occurrence() {
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0x1000,
        bytes: &[0; 16],
    };
    let mut log = RawLog::<NAVAL_PACK_N>::new(2, 32);
    assert!(log.take(&memory, 0x1000, 16, 1).is_some());
    assert!(log.take(&memory, 0x1000, 16, 1).is_some());
    assert_eq!(
        theatres::prior_roster_header(&log, 2, 0x1000),
        Err("reserved land prior roster header ambiguous")
    );
    assert_eq!(theatres::prior_roster_header(&log, 2, 0x2000), Ok(None));
    assert!(theatres::prior_roster_header(&log, 3, 0x1000).is_err());
}

#[test]
fn failed_selected_root_alias_survives_cold_and_cached_copy_and_decode() {
    struct FailOnce<'a> {
        inner: Observed<'a>,
        fault: usize,
        attempts: std::cell::Cell<usize>,
    }
    impl Memory for FailOnce<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == self.fault {
                let previous = self.attempts.get();
                self.attempts.set(previous + 1);
                if previous == 0 {
                    return false;
                }
            }
            self.inner.copy(address, output)
        }
    }
    let mut bytes = fixture(0);
    p64(&mut bytes, 0x11010, ZERO as u64);
    for cached in [false, true] {
        for (fault, reason) in [
            (ZERO, "reserved country object identity mismatch"),
            (
                ZERO + engine_native::ccountry::ID,
                "reserved country id is not zero",
            ),
        ] {
            let source = FailOnce {
                inner: Observed {
                    bytes: &bytes,
                    fault: None,
                    calls: RefCell::new(Vec::new()),
                },
                fault,
                attempts: std::cell::Cell::new(0),
            };
            let mut cache = engine_native_intelligence::country_memory::Cache::try_with_capacity(128)
                .expect("reserved-land fixture cache allocation");
            if cached {
                let view = cache.prepare(&source, kit::BASE, kit::GAME_STATE);
                assert_eq!(
                    view.seed_status(),
                    engine_native_intelligence::country_memory::SeedStatus::Unavailable { reason }
                );
            }
            let mut job = NavalJob::fixture();
            let ctx = NavalCopyContext {
                hour: 0,
                force_full: false,
                base: kit::BASE,
                game_state: kit::GAME_STATE,
            };
            let report = if cached {
                copy_naval_refresh(&cache.roster_view(&source), ctx, &mut job)
            } else {
                copy_naval_refresh(&source, ctx, &mut job)
            };
            assert!(report.raw_ok);
            assert!(!job.theatres_active);
            let observation = job
                .roster
                .reserved_observation(kit::BASE, kit::GAME_STATE)
                .unwrap();
            assert_eq!(observation.result(), Err(reason));
            assert_eq!(observation.physical_alias(ZERO), Some(Err(reason)));
            let live_calls = source.inner.calls.borrow().len();
            let mut captured = Captured::with_capacities(&Capacities::fixture());
            decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
            assert_eq!(captured.land.outcome, Outcome::Unavailable { reason });
            assert_eq!(captured.reserved_land_count(), None);
            assert_eq!(
                source.attempts.get(),
                1,
                "copy or decode retried selected identity"
            );
            assert_eq!(
                source.inner.calls.borrow().len(),
                live_calls,
                "decode followed native memory"
            );
            job.clear();
            assert_eq!(
                job.roster.reserved_observation(kit::BASE, kit::GAME_STATE),
                None
            );
        }
    }
}
