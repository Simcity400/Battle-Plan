//! State air bases' AirBaseRegistrations subtrees carried between hours
//! ([`super::RegistrationStore`], [`crate::air_base_carry_changes`]).
use super::air_bases_test_kit as kit;
use super::*;
use crate::{air_base_carry_changes::Watches, extended::fog, sampled_verify::Check};
use engine_native::{cair_base, cair_wing_pool, ccountry_air_container};
use std::cell::RefCell;

const B0_HANDLE: u64 = (0x41 << 32) | cair_base::TYPE as u64;

/// The fixture with B0's pool naming B0 by its base handle, as `SetAirBase` links it.
fn fixture() -> Vec<u8> {
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

/// A job carrying through a fresh store under `watches`.
fn job(watches: &'static Watches) -> AirBasesJob {
    let mut job = AirBasesJob::fixture();
    job.registration_store = Some(Box::new(RegistrationStore::new()));
    job.carry = CarryRun {
        changes: watches,
        ..CarryRun::default()
    };
    job
}

/// One acquisition of `bytes` under `check` and its replay: the
/// registrations, the live reads and the store's counts.
fn hour(
    bytes: &[u8],
    job: &mut AirBasesJob,
    check: Check,
) -> (fog::RegistrationsSnapshot, Vec<usize>, RegistrationCounts) {
    job.carry.check = check;
    let memory = Reads {
        inner: kit::memory(bytes),
        reads: RefCell::default(),
    };
    let report = copy_air_bases_refresh(
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
    (
        decoded.registrations_snapshot(),
        memory.reads.into_inner(),
        counts,
    )
}

fn direct(bytes: &[u8]) -> fog::RegistrationsSnapshot {
    kit::direct(&kit::memory(bytes)).regs
}

fn assert_same(left: &fog::RegistrationsSnapshot, right: &fog::RegistrationsSnapshot) {
    assert_eq!(
        (left.outcome, &left.rows, &left.wings),
        (right.outcome, &right.rows, &right.wings)
    );
}

fn counts(carried: u64, read: u64, verified: u64, mismatches: u64) -> RegistrationCounts {
    RegistrationCounts {
        carried,
        read,
        verified,
        mismatches,
    }
}

/// An hour after a base's fresh read carries its subtree without reading a
/// container, pool or wing, and the replay publishes the rows a fresh read
/// would; a hooked write to one base reads only that base again.
#[test]
fn an_untouched_state_base_carries_its_subtree_and_a_touched_one_is_read() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    let (first, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(0, 2, 0, 0));
    assert_same(&first, &direct(&bytes));
    let (carried, reads, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(2, 0, 0, 0));
    for object in [kit::CONT1, kit::POOL0, kit::W0, kit::W1] {
        assert!(
            !reads
                .iter()
                .any(|&at| (object..object + 0x100).contains(&at)),
            "read {object:#x}"
        );
    }
    assert_same(&carried, &first);

    // RemoveWing drops W1 from B0's pool, marking B0 by the pool's handle.
    let notification = watches.notify(Some(B0_HANDLE));
    // The wing vector's count is its header's last word.
    kit::put32(&mut bytes, kit::POOL0 + cair_wing_pool::WINGS + 12, 1);
    drop(notification);
    let (removed, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(1, 1, 0, 0));
    assert_same(&removed, &direct(&bytes));
    assert_eq!(removed.wings, [kit::WING_HANDLE_0]);
}

/// Each base's registration time is named by how its subtree was taken: the
/// host is read without a carry, the state bases are read and held, then
/// carried with the spans and bytes they splice.
#[test]
fn registration_work_names_each_base_by_how_its_subtree_was_taken() {
    let bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(job.registration_work.bases(), [1, 0, 2]);
    assert_eq!(job.registration_work.spliced(), [0, 0]);
    hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(job.registration_work.bases(), [1, 2, 0]);
    assert!(
        job.registration_work
            .spliced()
            .iter()
            .all(|&count| count > 0)
    );
}

/// A write no hook observed is carried until the base's slice reads it
/// afresh: the comparison counts it, poisons reuse and publishes the fresh
/// rows, and every later hour reads every base.
#[test]
fn a_compared_base_that_differs_poisons_reuse() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    hour(&bytes, &mut job, Check::Nothing);
    kit::put64(
        &mut bytes,
        kit::CONT1 + ccountry_air_container::CAPACITY_RAW,
        7,
    );
    // B0 differs and poisons at once; B1 then takes no certificate.
    let (compared, _, read) = hour(&bytes, &mut job, Check::Every);
    assert_eq!(read, counts(0, 2, 1, 1));
    assert!(watches.poisoned());
    assert_same(&compared, &direct(&bytes));
    let (after, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(0, 2, 0, 0));
    assert_same(&after, &direct(&bytes));
}

/// At attach no watch table is installed: the first pass holds nothing but
/// still counts the state bases it reached, which size the table, and the
/// hour after the table installs carries.
#[test]
fn the_first_pass_sizes_the_watches_without_holding_a_base() {
    let bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::without_table()));
    let mut job = job(watches);
    let (_, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(0, 2, 0, 0));
    let bases = job.registration_store.as_ref().unwrap().bases();
    assert_eq!(bases, 2);
    watches.offer(bases);
    watches.boundary(false);
    let (_, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(0, 2, 0, 0));
    let (_, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(2, 0, 0, 0));
}

/// A base listing a pool whose base handle names no base (or another one) is read every hour: that
/// pool's writers would not notify it.
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
    hour(&bytes, &mut job, Check::Nothing);
    let (carried, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(1, 1, 0, 0));
    assert_same(&carried, &direct(&bytes));
}

/// A base the dense slot check covers this hour is read afresh even when the
/// carry's own slice does not cover it, so the slot check still compares
/// the base it would otherwise carry.
#[test]
fn a_base_the_slot_check_covers_is_read_afresh() {
    // Each state base's dense list names its one container.
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
    job.slot_run = crate::air_base_changes::SlotRun {
        dense: true,
        check: Check::Nothing,
    };
    hour(&bytes, &mut job, Check::Nothing);
    let (_, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(2, 0, 0, 0));
    job.slot_run.check = Check::Every;
    let (checked, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(0, 2, 0, 0));
    // The host base, whose dense list is empty, is planned and compared too.
    assert_eq!(
        (
            job.slot_report.dense,
            job.slot_report.verified,
            job.slot_report.mismatches
        ),
        (3, 3, 0)
    );
    assert_same(&checked, &direct(&bytes));
}

/// A pass that stops early (here an unreadable air manager) holds nothing,
/// so the next complete pass reads every base.
#[test]
fn a_pass_that_stops_early_holds_nothing() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    hour(&bytes, &mut job, Check::Nothing);
    let manager = kit::GAME_STATE + engine_native::cgame_state::STRATEGIC_AIR_MANAGER;
    kit::put64(&mut bytes, manager, 0);
    hour(&bytes, &mut job, Check::Nothing);
    kit::put64(&mut bytes, manager, kit::MANAGER as u64);
    let (restored, _, read) = hour(&bytes, &mut job, Check::Nothing);
    assert_eq!(read, counts(0, 2, 0, 0));
    assert_same(&restored, &direct(&bytes));
}

/// One fresh read of a single span with `bytes`, reaching `objects` as its
/// containers and wings.
fn read<'a>(
    span: &'a crate::raw::Span,
    bytes: &'a [u8],
    objects: &'a [usize],
) -> super::registration_store::Fresh<'a> {
    super::registration_store::Fresh {
        spans: std::slice::from_ref(span),
        bytes,
        start: 0,
        links: &[],
        demand: [1, 0],
        link_table: 0..0,
        containers: objects,
        wings: objects,
    }
}

/// A container's span is held as the rows consume it: its fuel words, which
/// the hourly strategic-air stage rewrites without a registration writer,
/// never make a compared base differ, while its capacity does, named by the
/// span and its first differing byte.
#[test]
fn a_held_container_span_differs_only_in_what_the_rows_consume() {
    use super::registration_store::Difference;
    use engine_native::cair_base::CONTAINER_SPAN;
    let container = 0x1000;
    let span = crate::raw::Span {
        addr: container + ccountry_air_container::COUNTRY,
        len: CONTAINER_SPAN as u32,
        ok: true,
        mask: 0,
        start: 0,
        bulk: false,
    };
    let containers = [container];
    let mut store = RegistrationStore::new();
    store.begin();
    let mut bytes = vec![0_u8; CONTAINER_SPAN];
    let certificate = Some((1, 1));
    assert_eq!(
        store.hold(
            7,
            0x10,
            certificate,
            true,
            &read(&span, &bytes, &containers),
            false
        ),
        None
    );
    let offset = |field: usize| field - ccountry_air_container::COUNTRY;
    bytes[offset(ccountry_air_container::FUEL_DEMAND_RAW)] = 9;
    bytes[offset(ccountry_air_container::FUEL_ALLOCATED_RAW)] = 4;
    assert_eq!(
        store.hold(
            7,
            0x10,
            certificate,
            true,
            &read(&span, &bytes, &containers),
            true
        ),
        None
    );
    bytes[offset(ccountry_air_container::CAPACITY_RAW) + 1] = 3;
    assert_eq!(
        store.hold(
            7,
            0x10,
            certificate,
            true,
            &read(&span, &bytes, &containers),
            true
        ),
        Some(Difference {
            what: "span bytes",
            address: span.addr,
            len: span.len,
            byte: offset(ccountry_air_container::CAPACITY_RAW) + 1,
        })
    );
    assert_eq!(store.counts(), counts(0, 3, 2, 1));
}

/// A wing's identity is held as the walk consumes it: its `CSelectable` state (the selected byte
/// the player sets) never makes a compared base differ, while its handle does.
#[test]
fn a_held_wing_identity_differs_only_in_what_the_walk_consumes() {
    use super::registration_store::Difference;
    let wing = 0x2000;
    let span = crate::raw::Span {
        addr: wing,
        len: 32,
        ok: true,
        mask: 0,
        start: 0,
        bulk: false,
    };
    let wings = [wing];
    let mut store = RegistrationStore::new();
    store.begin();
    let mut bytes = vec![0_u8; 32];
    let certificate = Some((1, 1));
    assert_eq!(
        store.hold(
            7,
            0x10,
            certificate,
            true,
            &read(&span, &bytes, &wings),
            false
        ),
        None
    );
    bytes[0x0c] = 1;
    assert_eq!(
        store.hold(
            7,
            0x10,
            certificate,
            true,
            &read(&span, &bytes, &wings),
            true
        ),
        None
    );
    bytes[24] = 5;
    assert_eq!(
        store.hold(
            7,
            0x10,
            certificate,
            true,
            &read(&span, &bytes, &wings),
            true
        ),
        Some(Difference {
            what: "span bytes",
            address: wing,
            len: 32,
            byte: 24,
        })
    );
}

/// [`hour`] decoding with `decoded`, which keeps walked bases across hours.
fn hour_reusing(
    bytes: &[u8],
    job: &mut AirBasesJob,
    check: Check,
    decoded: &mut DecodedRegistrations,
) -> fog::RegistrationsSnapshot {
    job.carry.check = check;
    let report = copy_air_bases_refresh(
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

fn decoded(reused: u64, walked: u64) -> DecodedCounts {
    DecodedCounts { reused, walked }
}

/// The decode worker appends a carried base's rows from its walk of the
/// same held segment, walks a base read afresh into a new segment, and keeps
/// appending a base whose fresh read compared equal to its segment.
#[test]
fn a_carried_segment_appends_the_rows_its_earlier_walk_decoded() {
    let mut bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    let mut reuse = DecodedRegistrations::default();
    let first = hour_reusing(&bytes, &mut job, Check::Nothing, &mut reuse);
    assert_same(&first, &direct(&bytes));
    assert_eq!(job.segments.len(), 2);
    assert_eq!(reuse.counts(), decoded(0, 2));

    let carried = hour_reusing(&bytes, &mut job, Check::Nothing, &mut reuse);
    assert_eq!(reuse.counts(), decoded(2, 2));
    assert_same(&carried, &first);

    // RemoveWing drops W1 from B0's pool: B0 is read into a new segment and
    // walked; B1 still appends.
    let notification = watches.notify(Some(B0_HANDLE));
    kit::put32(&mut bytes, kit::POOL0 + cair_wing_pool::WINGS + 12, 1);
    drop(notification);
    let removed = hour_reusing(&bytes, &mut job, Check::Nothing, &mut reuse);
    assert_eq!(reuse.counts(), decoded(3, 3));
    assert_same(&removed, &direct(&bytes));
    assert_eq!(removed.wings, [kit::WING_HANDLE_0]);

    // Every base is read and compared equal: both keep their segments.
    let compared = hour_reusing(&bytes, &mut job, Check::Every, &mut reuse);
    assert_eq!(
        job.registration_store.as_ref().unwrap().counts(),
        counts(0, 2, 2, 0)
    );
    assert_eq!(reuse.counts(), decoded(5, 3));
    assert_same(&compared, &direct(&bytes));
}

/// A base the acquisition reports no segment for is walked every hour, and
/// a pass that cannot index the segments walks every base.
#[test]
fn a_base_without_a_reported_segment_is_walked() {
    let bytes = fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = job(watches);
    let mut reuse = DecodedRegistrations::default();
    hour_reusing(&bytes, &mut job, Check::Nothing, &mut reuse);
    job.registration_store = None;
    let walked = hour_reusing(&bytes, &mut job, Check::Nothing, &mut reuse);
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
    // The complete-slot fallback retains dense repeats, which cannot use the
    // fast path's proof of distinct occupied slots.
    job.slot_run.dense = false;
    let source = kit::direct(&kit::memory(&bytes)).bases;
    let decode = |job: &AirBasesJob| {
        let mut output = carrier::Bases::new(8);
        output.prepare_states(job);
        output.decode_states(job, kit::BASE, &source, &mut [None; 16]);
        output.packet(B0_HANDLE)
    };
    let acquire = |job: &mut AirBasesJob| {
        let memory = Reads {
            inner: kit::memory(&bytes),
            reads: RefCell::default(),
        };
        let report = copy_air_bases_refresh_growing(
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
        copy_air_bases_refresh_growing(
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
    assert!(copy_air_bases_refresh_growing(&memory, ctx(), &mut job).raw_ok);
    assert_eq!(job.registration_store.as_ref().unwrap().counts().carried, 2);
    assert!(!memory.reads.borrow().contains(&DENSE));
    let source = kit::direct(&kit::memory(&bytes)).bases;
    let mut output = carrier::Bases::new(8);
    output.prepare_states(&job);
    output.decode_states(&job, kit::BASE, &source, &mut [None; 16]);
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
