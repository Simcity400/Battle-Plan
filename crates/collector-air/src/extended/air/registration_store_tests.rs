use super::air_bases_test_kit as kit;
use super::*;
use crate::extended::fog;
#[cfg(test)]
use collector_observers::air_base_carry_changes::Watches;
use engine_native_parts::cair_wing_pool;
use std::cell::RefCell;

pub const B0_HANDLE: u64 = (0x41 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;

#[cfg(all(test, feature = "air_base_changes"))]
#[test]
fn ordinary_air_base_refresh_selects_dense_slots() {
    let bytes = fixture();
    let mut job = AirBasesJob::fixture();
    let report = copy_air_bases_refresh(
        &kit::memory(&bytes),
        AirBasesCopyContext {
            hour: 0,
            force_full: false,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert!(job.slot_run.dense);
}

pub fn fixture() -> Vec<u8> {
    let mut bytes = kit::fixture();
    kit::put64(
        &mut bytes,
        kit::POOL0 + cair_wing_pool::AIR_BASE_REFERENCE,
        B0_HANDLE,
    );
    bytes
}

struct Reads<'a> {
    inner: kit::SyntheticMemory<'a>,
    reads: RefCell<Vec<usize>>,
}

impl Memory for Reads<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.inner.copy(address, output)
    }
}

#[cfg(test)]
fn job(watches: &'static Watches) -> AirBasesJob {
    let mut job = AirBasesJob::fixture();
    job.registration_store = Some(Box::new(RegistrationStore::new()));
    job.carry = CarryRun {
        changes: watches,
        ..CarryRun::default()
    };
    job
}

#[cfg(test)]
fn hour(
    bytes: &[u8],
    job: &mut AirBasesJob,
) -> (fog::RegistrationsSnapshot, Vec<usize>, RegistrationCounts) {
    let (decoded, reads, counts) = captured_hour(bytes, job);
    (decoded.registrations_snapshot(), reads, counts)
}

pub fn captured_hour(
    bytes: &[u8],
    job: &mut AirBasesJob,
) -> (fog::Captured, Vec<usize>, RegistrationCounts) {
    let memory = Reads {
        inner: kit::memory(bytes),
        reads: RefCell::default(),
    };
    let report = copy_air_bases_fixture(
        &memory,
        AirBasesCopyContext {
            hour: 0,
            force_full: false,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        },
        job,
    );
    assert!(report.raw_ok);
    let mut decoded = fog::Captured::with_capacities(&fog::Capacities {
        rows: [4, 4, 4, 4, 4, 64, 4],
        radar_provinces: 8,
        area_provinces: 8,
        wings: 64,
    });
    decoded.registrations_raw_mode = true;
    decoded.decode_registrations_payload(kit::BASE, job, &mut [0; AIR_BASES_N]);
    let counts = job.registration_store.as_ref().unwrap().counts();
    (decoded, memory.reads.into_inner(), counts)
}

pub fn direct(bytes: &[u8]) -> fog::RegistrationsSnapshot {
    kit::direct(&kit::memory(bytes)).regs
}

pub fn assert_same(left: &fog::RegistrationsSnapshot, right: &fog::RegistrationsSnapshot) {
    assert_eq!(
        (left.outcome, &left.rows, &left.wings),
        (right.outcome, &right.rows, &right.wings)
    );
}

pub fn counts(carried: u64, read: u64) -> RegistrationCounts {
    RegistrationCounts { carried, read }
}

#[test]
fn an_untouched_state_base_carries_its_subtree_and_a_touched_one_is_read() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    let (first, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(0, 2));
    assert_same(&first, &direct(&bytes));
    let (carried, reads, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(2, 0));
    for object in [kit::CONT1, kit::POOL0, kit::W0, kit::W1] {
        assert!(
            !reads
                .iter()
                .any(|&at| (object..object + 0x100).contains(&at)),
            "read {object:#x}"
        );
    }
    assert_same(&carried, &first);

    let notification = watches.notify(Some(B0_HANDLE));
    kit::put32(&mut bytes, kit::POOL0 + cair_wing_pool::WINGS + 12, 1);
    drop(notification);
    let (removed, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(1, 1));
    assert_same(&removed, &direct(&bytes));
    assert_eq!(removed.wings, [kit::WING_HANDLE_0]);
}

#[test]
fn registration_work_names_each_base_by_how_its_subtree_was_taken() {
    let bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    hour(&bytes, &mut job);
    assert_eq!(job.registration_work.bases(), [1, 0, 2]);
    assert_eq!(job.registration_work.spliced(), [0, 0]);
    hour(&bytes, &mut job);
    assert_eq!(job.registration_work.bases(), [1, 2, 0]);
    assert!(
        job.registration_work
            .spliced()
            .iter()
            .all(|&count| count > 0)
    );
}

#[test]
fn the_first_pass_sizes_the_watches_without_holding_a_base() {
    let bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::without_table()));
    let mut job = job(watches);
    let (_, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(0, 2));
    let bases = job.registration_store.as_ref().unwrap().bases();
    assert_eq!(bases, 2);
    watches.offer(bases);
    watches.boundary(false);
    let (_, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(0, 2));
    let (_, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(2, 0));
}

#[test]
fn a_base_whose_pool_names_another_base_is_never_carried() {
    let mut bytes = fixture();
    kit::put64(
        &mut bytes,
        kit::POOL0 + cair_wing_pool::AIR_BASE_REFERENCE,
        0,
    );
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    hour(&bytes, &mut job);
    let (carried, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(1, 1));
    assert_same(&carried, &direct(&bytes));
}

#[test]
fn a_dense_planned_base_carries_its_subtree() {
    let mut bytes = fixture();
    for (base, list, container) in [
        (kit::B0, 0x68000, kit::CONT1),
        (kit::B1, 0x68100, kit::CONT2),
    ] {
        let header = base + cair_base::REGISTERED_CONTAINERS;
        kit::put64(&mut bytes, header, list as u64);
        kit::put32(&mut bytes, header + 8, 1);
        kit::put32(&mut bytes, header + 12, 1);
        kit::put64(&mut bytes, list, container as u64);
    }
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    job.slot_run = crate::air_base_changes::SlotRun { dense: true };
    let (first, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(0, 2));
    assert_eq!(job.slot_report.dense, 3);
    assert_same(&first, &direct(&bytes));
    let (carried, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(2, 0));
    assert_same(&carried, &first);
}

#[test]
fn a_pass_that_stops_early_holds_nothing() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    hour(&bytes, &mut job);
    let manager = kit::GAME_STATE + engine_native::cgame_state::STRATEGIC_AIR_MANAGER;
    kit::put64(&mut bytes, manager, 0);
    hour(&bytes, &mut job);
    kit::put64(&mut bytes, manager, kit::MANAGER as u64);
    let (restored, _, read) = hour(&bytes, &mut job);
    assert_eq!(read, counts(0, 2));
    assert_same(&restored, &direct(&bytes));
}

#[cfg(test)]
fn hour_reusing(
    bytes: &[u8],
    job: &mut AirBasesJob,
    decoded: &mut DecodedRegistrations,
) -> fog::RegistrationsSnapshot {
    let report = copy_air_bases_fixture(
        &kit::memory(bytes),
        AirBasesCopyContext {
            hour: 0,
            force_full: false,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        },
        job,
    );
    assert!(report.raw_ok);
    let mut captured = fog::Captured::with_capacities(&fog::Capacities {
        rows: [4, 4, 4, 4, 4, 64, 4],
        radar_provinces: 8,
        area_provinces: 8,
        wings: 64,
    });
    captured.registrations_raw_mode = true;
    let reuse = decoded.begin(&job.segments);
    captured.decode_registrations_with_carrier_payload(
        kit::BASE,
        job,
        &[],
        &mut super::carrier::Bases::new(0),
        &mut [0; AIR_BASES_N],
        reuse.then_some(decoded as &mut dyn cair_base::RegistrationReuse),
    );
    decoded.finish();
    captured.registrations_snapshot()
}

#[cfg(test)]
fn decoded(reused: u64, walked: u64) -> DecodedCounts {
    DecodedCounts { reused, walked }
}

#[test]
fn a_carried_segment_appends_the_rows_its_earlier_walk_decoded() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    let mut reuse = DecodedRegistrations::default();
    let first = hour_reusing(&bytes, &mut job, &mut reuse);
    assert_same(&first, &direct(&bytes));
    assert_eq!(job.segments.len(), 2);
    assert_eq!(reuse.counts(), decoded(0, 2));

    let carried = hour_reusing(&bytes, &mut job, &mut reuse);
    assert_eq!(reuse.counts(), decoded(2, 2));
    assert_same(&carried, &first);

    let notification = watches.notify(Some(B0_HANDLE));
    kit::put32(&mut bytes, kit::POOL0 + cair_wing_pool::WINGS + 12, 1);
    drop(notification);
    let removed = hour_reusing(&bytes, &mut job, &mut reuse);
    assert_eq!(reuse.counts(), decoded(3, 3));
    assert_same(&removed, &direct(&bytes));
    assert_eq!(removed.wings, [kit::WING_HANDLE_0]);
}

#[test]
fn a_base_without_a_reported_segment_is_walked() {
    let bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    let mut reuse = DecodedRegistrations::default();
    hour_reusing(&bytes, &mut job, &mut reuse);
    job.registration_store = None;
    let walked = hour_reusing(&bytes, &mut job, &mut reuse);
    assert!(job.segments.is_empty());
    assert_eq!(reuse.counts(), decoded(0, 2));
    assert_same(&walked, &direct(&bytes));
}

#[test]
fn state_summary_countries_use_the_held_order_without_quiet_native_reads() {
    const DENSE: usize = 0x68000;
    let mut bytes = fixture();
    kit::put64(
        &mut bytes,
        kit::B0 + cair_base::REGISTERED_CONTAINERS,
        DENSE as u64,
    );
    kit::put32(
        &mut bytes,
        kit::B0 + cair_base::REGISTERED_CONTAINERS + 8,
        2,
    );
    kit::put32(
        &mut bytes,
        kit::B0 + cair_base::REGISTERED_CONTAINERS + 12,
        2,
    );
    kit::put64(&mut bytes, DENSE, kit::CONT1 as u64);
    kit::put64(&mut bytes, DENSE + 8, kit::CONT1 as u64);
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    job.slot_run.dense = false;
    let source = kit::direct(&kit::memory(&bytes)).bases;
    let decode = |job: &AirBasesJob| {
        let mut output = carrier::Bases::new(8);
        output.prepare_states(job).unwrap();
        output.decode_states(job, kit::BASE, &source, &mut [None; 16]).unwrap();
        output.packet(B0_HANDLE)
    };
    let acquire = |job: &mut AirBasesJob| {
        let memory = Reads {
            inner: kit::memory(&bytes),
            reads: RefCell::default(),
        };
        let report = copy_air_bases_fixture(
            &memory,
            AirBasesCopyContext {
                hour: 0,
                force_full: false,
                base: kit::BASE,
                game_state: kit::GAME_STATE,
            },
            job,
        );
        assert!(report.raw_ok);
        memory.reads.into_inner()
    };
    acquire(&mut job);
    let first = decode(&job);
    assert_eq!(
        first
            .require("summary")
            .unwrap()
            .as_ref()
            .unwrap()
            .registered_countries
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    let reads = acquire(&mut job);
    assert_eq!(job.registration_store.as_ref().unwrap().counts().carried, 2);
    assert!(!reads.contains(&DENSE));
    assert!(!reads.contains(&(kit::B0 + cair_base::REGISTERED_CONTAINERS)));
    assert_eq!(decode(&job), first);
}

#[test]
fn failed_optional_state_order_stays_unknown_under_the_registration_certificate() {
    const DENSE: usize = 0x68000;
    struct Failed<'a> {
        memory: kit::SyntheticMemory<'a>,
    }
    impl Memory for Failed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            address != DENSE && self.memory.copy(address, output)
        }
    }
    let mut bytes = fixture();
    kit::put64(
        &mut bytes,
        kit::B0 + cair_base::REGISTERED_CONTAINERS,
        DENSE as u64,
    );
    kit::put32(
        &mut bytes,
        kit::B0 + cair_base::REGISTERED_CONTAINERS + 8,
        1,
    );
    kit::put32(
        &mut bytes,
        kit::B0 + cair_base::REGISTERED_CONTAINERS + 12,
        1,
    );
    kit::put64(&mut bytes, DENSE, kit::CONT1 as u64);
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    job.slot_run.dense = false;
    let ctx = || AirBasesCopyContext {
        hour: 0,
        force_full: false,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    assert!(
        copy_air_bases_fixture(
            &Failed {
                memory: kit::memory(&bytes)
            },
            ctx(),
            &mut job
        )
        .raw_ok
    );
    assert!(
        job.segments
            .iter()
            .any(|segment| segment.handle == B0_HANDLE)
    );
    let memory = Reads {
        inner: kit::memory(&bytes),
        reads: RefCell::default(),
    };
    assert!(copy_air_bases_fixture(&memory, ctx(), &mut job).raw_ok);
    assert_eq!(job.registration_store.as_ref().unwrap().counts().carried, 2);
    assert!(!memory.reads.borrow().contains(&DENSE));
    let source = kit::direct(&kit::memory(&bytes)).bases;
    let mut output = carrier::Bases::new(8);
    output.prepare_states(&job).unwrap();
    output.decode_states(&job, kit::BASE, &source, &mut [None; 16]).unwrap();
    assert!(
        output
            .packet(B0_HANDLE)
            .require("summary")
            .unwrap()
            .as_ref()
            .unwrap()
            .registered_countries
            .is_none()
    );
}
