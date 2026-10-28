//! The AirBases census of state bases carried between hours
//! ([`super::census_store`], [`crate::air_base_carry_changes`]).
use super::air_bases_test_kit as kit;
use super::*;
use crate::air_base_carry_changes::Watches;
use engine_mutations::air_base_mutations::MUTATIONS;
use engine_native::{cair_base, cprovince, cstate, walk::Outcome};
use std::cell::RefCell;

const B0_HANDLE: u64 = (0x41 << 32) | cair_base::TYPE as u64;
const B1_HANDLE: u64 = (0x42 << 32) | cair_base::TYPE as u64;

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

fn new_job(watches: &'static Watches) -> AirBasesJob {
    let mut job = AirBasesJob::fixture();
    job.registration_store = Some(Box::new(RegistrationStore::new()));
    job.carry = CarryRun {
        changes: watches,
        ..CarryRun::default()
    };
    job
}

fn rows(split: &kit::Split) -> Vec<cair_base::Row> {
    let Outcome::Rows { len } = split.bases_outcome else {
        panic!("{:?}", split.bases_outcome);
    };
    split.bases[..len].to_vec()
}

/// One acquisition of `bytes` and its replay: the AirBases rows, the live
/// reads and what the census carried and read.
fn hour(
    bytes: &[u8],
    job: &mut AirBasesJob,
) -> (Vec<cair_base::Row>, Vec<usize>, census_store::Counts) {
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
    let counts = job.registration_store.as_ref().unwrap().census_counts();
    (rows(&kit::decode(job)), memory.reads.into_inner(), counts)
}

fn direct(bytes: &[u8]) -> Vec<cair_base::Row> {
    rows(&kit::direct(&kit::memory(bytes)))
}

/// Every read a state base's row takes: identity, host pack, state identity,
/// province identity and state link.
fn row_reads(base: usize, state: usize, province: usize) -> [usize; 7] {
    [
        base,
        base + cair_base::HOST_REFERENCE,
        state,
        state + cstate::ID,
        province,
        province + cprovince::STATE,
        province + cprovince::ID,
    ]
}

fn b0() -> [usize; 7] {
    row_reads(kit::B0, kit::S0, kit::P0)
}

fn b1() -> [usize; 7] {
    row_reads(kit::B1, kit::S1, kit::P1)
}

/// The ordinary state base slot words.
fn slot_words() -> [usize; 3] {
    [kit::SVEC, kit::SVEC + 8, kit::SVEC + 16]
}

fn read_any(reads: &[usize], addresses: &[usize]) -> bool {
    addresses.iter().any(|address| reads.contains(address))
}

fn read_all(reads: &[usize], addresses: &[usize]) -> bool {
    addresses.iter().all(|address| reads.contains(address))
}

fn counts(arrays: [u64; 2], rows: [u64; 2]) -> ([u64; 2], [u64; 2]) {
    (arrays, rows)
}

fn of(counts: census_store::Counts) -> ([u64; 2], [u64; 2]) {
    (counts.arrays, counts.rows)
}

fn site(name: &str) -> usize {
    MUTATIONS
        .iter()
        .position(|(site, _)| site.name == name)
        .unwrap()
}

/// A quiet hour reads no slot word and no state base row, even when the game
/// changed them without a hooked writer: it publishes the held rows. The
/// stateless base is read every hour.
#[test]
fn a_quiet_hour_reads_no_carried_census_field_even_when_memory_changed() {
    let mut bytes = kit::fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = new_job(watches);
    let (first, reads, census) = hour(&bytes, &mut job);
    assert_eq!(first, direct(&bytes));
    assert_eq!(of(census), counts([0, 3], [0, 2]));
    assert!(read_all(&reads, &slot_words()));
    assert!(read_all(&reads, &b0()) && read_all(&reads, &b1()));

    kit::put32(&mut bytes, kit::B0 + cair_base::CAPACITY, 999);
    kit::put32(&mut bytes, kit::B1 + cair_base::KIND, 2);
    kit::put32(&mut bytes, kit::S0 + cstate::ID, 9);
    kit::put32(&mut bytes, kit::P1 + cprovince::ID, 99);
    kit::put64(&mut bytes, kit::SVEC + 16, 0);
    let (quiet, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([3, 0], [2, 0]));
    assert!(!read_any(&reads, &slot_words()));
    assert!(!read_any(&reads, &b0()) && !read_any(&reads, &b1()));
    assert!(reads.contains(&kit::HVEC) && reads.contains(&kit::H0));
    assert_eq!(
        census.spans[0],
        6 + 1 + 2,
        "manager, headers, host slot and row"
    );
    assert!(census.spans[1] > 0);
    assert_eq!(quiet, first);
}

/// Each writer rereads exactly what it names: capacity refresh its base's
/// row, a state base creator the slot words, and destruction both.
#[test]
fn each_writer_rereads_exactly_what_it_names() {
    let mut bytes = kit::fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = new_job(watches);
    hour(&bytes, &mut job);

    let refresh = watches.notify(Some(B0_HANDLE));
    kit::put32(&mut bytes, kit::B0 + cair_base::CAPACITY, 1500);
    drop(refresh);
    let (refreshed, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([3, 0], [1, 1]));
    assert!(read_all(&reads, &b0()));
    assert!(!read_any(&reads, &b1()) && !read_any(&reads, &slot_words()));
    assert_eq!(refreshed, direct(&bytes));
    assert_eq!(refreshed[0].capacity, 1500);

    drop(watches.notify_site(site("ensure_state_air_base"), 0));
    let (created, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([0, 3], [2, 0]));
    assert!(read_all(&reads, &slot_words()));
    assert!(!read_any(&reads, &b0()) && !read_any(&reads, &b1()));
    assert_eq!(created, direct(&bytes));

    // As the forwarded destruction of B1 does, notify its handle and the
    // slots before its caller empties its slot.
    let destroyed = (
        watches.notify(Some(B1_HANDLE)),
        watches.notify_site(site("ensure_gun_air_base"), 0),
    );
    kit::put64(&mut bytes, kit::SVEC + 16, 0);
    drop(destroyed);
    let (emptied, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([0, 3], [1, 0]));
    assert!(read_all(&reads, &slot_words()));
    assert!(!read_any(&reads, &b0()) && !read_any(&reads, &b1()));
    assert_eq!(emptied, direct(&bytes));
    assert_eq!(emptied.len(), 2);
}

/// A new base is read while the others carry; an invalidating hour, a
/// forgotten store and a disabled family read the whole census.
#[test]
fn a_new_base_an_invalidating_hour_and_a_disabled_family_read_the_census() {
    let mut bytes = kit::fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = new_job(watches);
    hour(&bytes, &mut job);
    hour(&bytes, &mut job);

    let created = watches.notify_site(site("ensure_rocket_air_base"), 0);
    kit::add_alias_and_rocket(&mut bytes);
    drop(created);
    let (added, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([0, 3], [2, 1]));
    assert!(!read_any(&reads, &b0()) && !read_any(&reads, &b1()));
    assert_eq!(added, direct(&bytes));
    assert_eq!(added.len(), 4);
    let (_, _, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([3, 0], [3, 0]));

    watches.boundary(true);
    let (invalidated, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([0, 3], [0, 3]));
    assert!(read_all(&reads, &b0()) && read_all(&reads, &b1()));
    assert_eq!(invalidated, direct(&bytes));

    job.registration_store.as_mut().unwrap().forget();
    let (_, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([0, 3], [0, 3]));
    assert!(read_all(&reads, &slot_words()) && read_all(&reads, &b0()));

    let disabled: &'static Watches = Box::leak(Box::new(Watches::new()));
    let mut job = new_job(disabled);
    for _ in 0..2 {
        let (read, reads, census) = hour(&bytes, &mut job);
        assert_eq!(of(census), counts([0, 3], [0, 3]));
        assert!(read_all(&reads, &slot_words()));
        assert!(read_all(&reads, &b0()) && read_all(&reads, &b1()));
        assert_eq!(read, direct(&bytes));
    }
}

/// A slot array whose header moved is read even under an unchanged
/// certificate.
#[test]
fn a_moved_slot_array_is_read() {
    let mut bytes = kit::fixture();
    let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(16)));
    let mut job = new_job(watches);
    hour(&bytes, &mut job);
    const MOVED: usize = 0x31800;
    for index in 0..3 {
        let word = u64::from_le_bytes(
            bytes[kit::SVEC + index * 8..kit::SVEC + index * 8 + 8]
                .try_into()
                .unwrap(),
        );
        kit::put64(&mut bytes, MOVED + index * 8, word);
    }
    let header = kit::MANAGER + engine_native::cstrategic_air_manager::STATE_AIR_BASES;
    kit::put64(&mut bytes, header, MOVED as u64);
    let (moved, reads, census) = hour(&bytes, &mut job);
    assert_eq!(of(census), counts([2, 1], [2, 0]));
    assert!(read_all(&reads, &[MOVED, MOVED + 8, MOVED + 16]));
    assert_eq!(moved, direct(&bytes));
}
