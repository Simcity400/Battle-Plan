//! Factions and ordered membership acquired within the callback's country lease.
//! Each table retains its own independent consistency window and paired storage.

use crate::{
    acquisition_gather::{Buffers, Report as GatherReport, Request as GatherRequest},
    sampling::{mismatches, qpc},
};
use engine::{Memory, cfaction, cfaction_system, country_memory::Cache, walk::Outcome};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Names {
    rows: Vec<cfaction::Row>,
    bytes: Vec<u8>,
    bytes_len: usize,
    required_bytes: usize,
}

impl Names {
    pub(crate) fn take(rows: &mut Vec<cfaction::Row>, bytes: &mut Vec<u8>) -> Self {
        Self {
            rows: std::mem::take(rows),
            bytes: std::mem::take(bytes),
            bytes_len: 0,
            required_bytes: 0,
        }
    }

    fn restore(self, rows: &mut Vec<cfaction::Row>, bytes: &mut Vec<u8>) {
        *rows = self.rows;
        *bytes = self.bytes;
    }

    fn walk(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
        let names = cfaction::walk(memory, base, game_state, &mut self.rows, &mut self.bytes);
        self.bytes_len = names.bytes_len;
        self.required_bytes = names.required_bytes;
        names.outcome
    }
}

pub(crate) struct Request {
    pub countries: Arc<Cache>,
    pub factions: GatherRequest<Names>,
    pub members: GatherRequest<Vec<cfaction_system::MemberRow>>,
}

impl Request {
    /// Return every allocation, including closed-window storage and partial
    /// observations after a panic. Dropping the request also returns its country
    /// lease; only typed rows and literal bytes remain in the frame.
    pub(crate) fn restore(
        self,
        factions: &mut Vec<cfaction::Row>,
        factions_second: &mut Vec<cfaction::Row>,
        names: &mut Vec<u8>,
        names_second: &mut Vec<u8>,
        members: &mut Vec<cfaction_system::MemberRow>,
        members_second: &mut Vec<cfaction_system::MemberRow>,
    ) -> (usize, usize) {
        let Buffers { first, second } = self.factions.buffers;
        let lengths = (first.bytes_len, first.required_bytes);
        first.restore(factions, names);
        second.restore(factions_second, names_second);
        self.members.buffers.restore(members, members_second);
        lengths
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub factions: GatherReport,
    pub members: GatherReport,
    /// Paired walk and comparison ticks for Factions, then FactionMembers.
    /// A failed runner has no timing report; zero then means unmeasured work.
    pub ticks: [u64; 2],
}

impl Report {
    pub(crate) fn failed(needed: [bool; 2], reason: &'static str) -> Self {
        let [factions, members] = needed.map(|needed| GatherReport {
            result: Outcome::Unavailable { reason },
            second: Outcome::Unavailable { reason },
            mismatches: usize::from(needed),
        });
        Self {
            factions,
            members,
            ticks: [0; 2],
        }
    }
}

fn name_mismatches(first: Outcome, names: &Names, second: Outcome, other: &Names) -> usize {
    mismatches(first, &names.rows, second, &other.rows)
        + usize::from(
            names.bytes_len != other.bytes_len
                || names.required_bytes != other.required_bytes
                || names.bytes[..names.bytes_len] != other.bytes[..other.bytes_len],
        )
}

pub(crate) fn run(request: &mut Request, memory: &impl Memory) -> Report {
    let cached = request.countries.view(memory);
    let names = &mut request.factions;
    let started = qpc();
    let factions = names.buffers.run(
        names.needed,
        |buffers, _| buffers.walk(&cached, names.base, names.game_state),
        name_mismatches,
    );
    let faction_ticks = qpc().saturating_sub(started).max(0) as u64;
    let members = &mut request.members;
    let started = qpc();
    let members = members.buffers.rows(members.needed, |rows, _| {
        cfaction_system::walk_members(&cached, members.base, members.game_state, rows)
    });
    Report {
        factions,
        members,
        ticks: [faction_ticks, qpc().saturating_sub(started).max(0) as u64],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{acquisition, acquisition_jobs, sampling::full_match};
    use engine::{ccountry, cdiplomacy_status, cgame_state, memory::SliceMemory, msvc_string};
    use std::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const SYSTEM: usize = 0x2200;
    const FACTIONS: usize = 0x2300;
    const FIRST: usize = 0x3000;
    const SECOND: usize = 0x3800;
    const MEMBERS: usize = 0x4300;
    const C1: usize = 0x5000;
    const C2: usize = 0x7000;
    const D1: usize = 0x9000;
    const D2: usize = 0xa000;
    const FIRST_ID: u64 = (4_u64 << 32) | 88;
    const SECOND_ID: u64 = 1_u64 << 32;
    const NAME: [u8; 4] = [b'f', 0, 0xff, b'n'];

    fn put32(bytes: &mut [u8], address: usize, value: u32) {
        bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], address: usize, value: u64) {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
        put64(bytes, address, data as u64);
        put32(bytes, address + 8, count);
        put32(bytes, address + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xb000];
        array(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 4);
        array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 8);
        // Canonical country one has native reference seven and an alias slot.
        for (slot, country) in [(1, C1), (2, C2), (3, C1)] {
            put64(&mut bytes, COUNTRIES + slot * 8, country as u64);
        }
        for (raw, canonical) in [(1, 1), (2, 2), (3, 1), (7, 1)] {
            put32(&mut bytes, LINKS + raw * 4, canonical);
        }
        for (country, id, diplomacy) in [(C1, 7, D1), (C2, 2, D2)] {
            put64(&mut bytes, country, ccountry::VTABLE_RVA as u64);
            put32(&mut bytes, country + ccountry::ID, id);
            put64(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            put64(&mut bytes, diplomacy, cdiplomacy_status::VTABLE_RVA as u64);
            put64(
                &mut bytes,
                diplomacy + cdiplomacy_status::COUNTRY,
                country as u64,
            );
            put64(
                &mut bytes,
                diplomacy + cdiplomacy_status::FACTION,
                FIRST as u64,
            );
        }
        put64(&mut bytes, GS + cgame_state::FACTION_SYSTEM, SYSTEM as u64);
        put64(&mut bytes, SYSTEM, cfaction_system::VTABLE_RVA as u64);
        array(&mut bytes, SYSTEM + cfaction_system::FACTIONS, FACTIONS, 2);
        put64(&mut bytes, FACTIONS, FIRST as u64);
        put64(&mut bytes, FACTIONS + 8, SECOND as u64);
        for (faction, id) in [(FIRST, FIRST_ID), (SECOND, SECOND_ID)] {
            put64(&mut bytes, faction, cfaction::VTABLE_RVA as u64);
            put64(&mut bytes, faction + cfaction::REFERENCE, id);
            put64(
                &mut bytes,
                faction + cfaction::NAME + msvc_string::CAPACITY,
                15,
            );
        }
        bytes[FIRST + cfaction::NAME..FIRST + cfaction::NAME + NAME.len()].copy_from_slice(&NAME);
        put64(
            &mut bytes,
            FIRST + cfaction::NAME + msvc_string::LENGTH,
            NAME.len() as u64,
        );
        // Country two leads; the second faction is valid and empty.
        array(&mut bytes, FIRST + cfaction::MEMBERS, MEMBERS, 2);
        put64(&mut bytes, MEMBERS, C2 as u64);
        put64(&mut bytes, MEMBERS + 8, C1 as u64);
        bytes
    }

    fn snapshot(memory: &impl Memory, cells: usize) -> Arc<Cache> {
        let mut storage = Arc::new(Cache::with_capacity(cells));
        Cache::prepare_shared(&mut storage, memory, 0, GS).unwrap()
    }

    struct Owner {
        factions: Vec<cfaction::Row>,
        factions_second: Vec<cfaction::Row>,
        names: Vec<u8>,
        names_second: Vec<u8>,
        members: Vec<cfaction_system::MemberRow>,
        members_second: Vec<cfaction_system::MemberRow>,
    }

    impl Owner {
        fn new(rows: usize, bytes: usize, members: usize) -> Self {
            Self {
                factions: vec![cfaction::Row::default(); rows],
                factions_second: vec![cfaction::Row::default(); rows],
                names: vec![0xa5; bytes],
                names_second: vec![0x5a; bytes],
                members: vec![cfaction_system::MemberRow::default(); members],
                members_second: vec![cfaction_system::MemberRow::default(); members],
            }
        }

        fn allocations(&self) -> [(usize, usize, usize); 6] {
            fn storage<T>(buffer: &Vec<T>) -> (usize, usize, usize) {
                (buffer.as_ptr() as usize, buffer.len(), buffer.capacity())
            }
            [
                storage(&self.factions),
                storage(&self.factions_second),
                storage(&self.names),
                storage(&self.names_second),
                storage(&self.members),
                storage(&self.members_second),
            ]
        }

        fn take(&mut self, countries: &Arc<Cache>, needed: [bool; 2]) -> Request {
            Request {
                countries: Arc::clone(countries),
                factions: GatherRequest {
                    base: 0,
                    game_state: GS,
                    needed: needed[0],
                    buffers: Buffers {
                        first: Names::take(&mut self.factions, &mut self.names),
                        second: Names::take(&mut self.factions_second, &mut self.names_second),
                    },
                },
                members: GatherRequest {
                    base: 0,
                    game_state: GS,
                    needed: needed[1],
                    buffers: Buffers::take(&mut self.members, &mut self.members_second),
                },
            }
        }

        fn restore(&mut self, request: Request) -> (usize, usize) {
            request.restore(
                &mut self.factions,
                &mut self.factions_second,
                &mut self.names,
                &mut self.names_second,
                &mut self.members,
                &mut self.members_second,
            )
        }
    }

    #[derive(Clone, Copy)]
    enum Mutation {
        None,
        NameBytes,
        RequiredBytes,
        NameFailure,
        MemberFailure,
        MemberOrder,
        NamePanic,
    }

    struct Observed<'a> {
        bytes: &'a [u8],
        roots: Cell<usize>,
        names: Cell<usize>,
        members: Cell<usize>,
        mutation: Mutation,
    }

    impl<'a> Observed<'a> {
        fn new(bytes: &'a [u8], mutation: Mutation) -> Self {
            Self {
                bytes,
                roots: Cell::new(0),
                names: Cell::new(0),
                members: Cell::new(0),
                mutation,
            }
        }

        fn counts(&self) -> [usize; 3] {
            [self.roots.get(), self.names.get(), self.members.get()]
        }
    }

    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == GS + cgame_state::FACTION_SYSTEM {
                self.roots.set(self.roots.get() + 1);
            }
            if !(SliceMemory {
                base: 0,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            let pass = self.roots.get();
            if address == FIRST + cfaction::NAME && output.len() == NAME.len() {
                self.names.set(self.names.get() + 1);
                match self.mutation {
                    Mutation::NameBytes if pass == 2 => output[0] ^= 1,
                    Mutation::NameFailure if pass == 1 => return false,
                    Mutation::NamePanic if pass == 2 => panic!("synthetic second name read panic"),
                    _ => {}
                }
            }
            if matches!(self.mutation, Mutation::RequiredBytes)
                && pass == 2
                && address == FIRST + cfaction::NAME + msvc_string::LENGTH
            {
                output.copy_from_slice(&5_u64.to_le_bytes());
            }
            if address == MEMBERS {
                self.members.set(self.members.get() + 1);
                if matches!(self.mutation, Mutation::MemberFailure) && pass == 3 {
                    return false;
                }
            }
            if matches!(self.mutation, Mutation::MemberOrder) && pass == 4 {
                if address == MEMBERS {
                    output.copy_from_slice(&(C1 as u64).to_le_bytes());
                } else if address == MEMBERS + 8 {
                    output.copy_from_slice(&(C2 as u64).to_le_bytes());
                }
            }
            true
        }
    }

    fn matched(report: GatherReport) -> bool {
        full_match(report.result, report.second, report.mismatches)
    }

    #[test]
    fn factions_owned_walk_matches_engine_rows_for_every_window_and_returns_all_storage() {
        for workers in [0, 2] {
            for needed in [[false, false], [true, false], [false, true], [true, true]] {
                for retired in [false, true] {
                    let bytes = fixture();
                    let memory = SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    };
                    let countries = snapshot(&memory, 512);
                    let cached = countries.view(&memory);
                    let mut direct = Owner::new(2, 16, 2);
                    let names =
                        cfaction::walk(&cached, 0, GS, &mut direct.factions, &mut direct.names);
                    let members =
                        cfaction_system::walk_members(&cached, 0, GS, &mut direct.members);
                    assert_eq!(names.outcome, Outcome::Rows { len: 2 });
                    assert_eq!(members, Outcome::Rows { len: 2 });
                    let mut owner = Owner::new(2, 16, 2);
                    if retired && !needed[0] {
                        owner.factions_second = Vec::new();
                        owner.names_second = Vec::new();
                    }
                    if retired && !needed[1] {
                        owner.members_second = Vec::new();
                    }
                    let allocations = owner.allocations();
                    let pool =
                        acquisition::Pool::start(workers, 1, move |request: &mut Request| {
                            let memory = Observed::new(&bytes, Mutation::None);
                            (run(request, &memory), memory.counts())
                        })
                        .unwrap();
                    let mut pending = Some(owner.take(&countries, needed));
                    assert!(
                        owner
                            .allocations()
                            .iter()
                            .all(|(_, len, cap)| *len == 0 && *cap == 0)
                    );
                    // SAFETY: the test owns synthetic source bytes through collection.
                    let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
                    let completion = pool.finish(ticket).unwrap();
                    let (report, counts) = completion.report.unwrap();
                    assert_eq!(report.factions.result, names.outcome);
                    assert_eq!(report.members.result, members);
                    assert_eq!([matched(report.factions), matched(report.members)], needed);
                    assert_eq!(
                        counts,
                        [
                            2 + usize::from(needed[0]) + usize::from(needed[1]),
                            1 + usize::from(needed[0]),
                            1 + usize::from(needed[1])
                        ]
                    );
                    assert_eq!(
                        owner.restore(completion.request),
                        (names.bytes_len, names.required_bytes)
                    );
                    assert_eq!(owner.allocations(), allocations);
                    assert_eq!(owner.factions, direct.factions);
                    assert_eq!(owner.members, direct.members);
                    assert_eq!(owner.names, direct.names);
                    assert_eq!(&owner.names[..4], &NAME);
                    assert_eq!(
                        owner
                            .members
                            .iter()
                            .map(|row| row.country)
                            .collect::<Vec<_>>(),
                        [2, 7]
                    );
                    if needed[0] {
                        assert_eq!(owner.factions_second, owner.factions);
                        assert_eq!(&owner.names_second[..4], &NAME);
                    } else {
                        assert!(
                            owner
                                .factions_second
                                .iter()
                                .all(|row| *row == cfaction::Row::default())
                        );
                        assert!(owner.names_second.iter().all(|byte| *byte == 0x5a));
                    }
                    if needed[1] {
                        assert_eq!(owner.members_second, owner.members);
                    } else {
                        assert!(
                            owner
                                .members_second
                                .iter()
                                .all(|row| *row == cfaction_system::MemberRow::default())
                        );
                    }
                    assert_eq!(Arc::strong_count(&countries), 1);
                    assert_eq!(pool.calibration().uncollected, 0);
                    pool.shutdown().unwrap();
                }
            }
        }
    }

    #[test]
    fn factions_second_walk_detects_exact_name_bytes_and_member_order_changes() {
        for mutation in [Mutation::NameBytes, Mutation::MemberOrder] {
            let bytes = fixture();
            let memory = Observed::new(&bytes, mutation);
            let countries = snapshot(&memory, 512);
            let mut owner = Owner::new(2, 16, 2);
            let mut request = owner.take(&countries, [true, true]);
            let report = run(&mut request, &memory);
            assert_eq!(memory.counts(), [4, 2, 2]);
            assert_eq!(report.factions.result, Outcome::Rows { len: 2 });
            assert_eq!(report.members.result, Outcome::Rows { len: 2 });
            match mutation {
                Mutation::NameBytes => {
                    assert_eq!(
                        request.factions.buffers.first.rows,
                        request.factions.buffers.second.rows
                    );
                    assert_eq!(report.factions.mismatches, 1);
                    assert!(!matched(report.factions));
                    assert!(matched(report.members));
                }
                Mutation::MemberOrder => {
                    assert!(matched(report.factions));
                    assert_eq!(report.members.mismatches, 2);
                    assert!(!matched(report.members));
                }
                _ => unreachable!(),
            }
            assert_eq!(owner.restore(request), (4, 4));
        }
    }

    #[test]
    fn factions_truncation_preserves_required_bytes_and_never_earns_window_credit() {
        for (rows, name_capacity, member_capacity) in [(2, 0, 2), (0, 16, 0), (2, 16, 1)] {
            let bytes = fixture();
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let countries = snapshot(&memory, 0); // The direct-reader capacity fallback is retained.
            let mut owner = Owner::new(rows, name_capacity, member_capacity);
            let allocations = owner.allocations();
            let mut request = owner.take(&countries, [true, true]);
            let report = run(&mut request, &memory);
            assert_eq!(report.factions.result, report.factions.second);
            assert_eq!(report.members.result, report.members.second);
            let name_len = usize::from(rows != 0 && name_capacity != 0) * 4;
            assert_eq!(owner.restore(request), (name_len, 4));
            assert_eq!(owner.allocations(), allocations);
            if rows == 0 || name_capacity == 0 {
                assert!(matches!(report.factions.result, Outcome::Truncated { .. }));
                assert!(!matched(report.factions));
            }
            if member_capacity < 2 {
                assert_eq!(
                    report.members.result,
                    Outcome::Truncated {
                        len: member_capacity,
                        unread_rows: (2 - member_capacity) as u32,
                    }
                );
                assert!(!matched(report.members));
            }
        }

        let bytes = fixture();
        let memory = Observed::new(&bytes, Mutation::RequiredBytes);
        let countries = snapshot(&memory, 512);
        let mut owner = Owner::new(2, 0, 2);
        let mut request = owner.take(&countries, [true, true]);
        let report = run(&mut request, &memory);
        let first = &request.factions.buffers.first;
        let second = &request.factions.buffers.second;
        assert_eq!(first.rows, second.rows);
        assert_eq!((first.bytes_len, second.bytes_len), (0, 0));
        assert_eq!((first.required_bytes, second.required_bytes), (4, 5));
        assert_eq!(
            report.factions.result,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(report.factions.result, report.factions.second);
        assert_eq!(
            report.factions.mismatches, 1,
            "omitted bytes still participate in consistency"
        );
        assert!(!matched(report.factions));
        assert!(matched(report.members));
        assert_eq!(owner.restore(request), (0, 4));
    }

    #[test]
    fn factions_failed_first_reads_do_not_suppress_independent_second_walks() {
        for mutation in [Mutation::NameFailure, Mutation::MemberFailure] {
            let bytes = fixture();
            let memory = Observed::new(&bytes, mutation);
            let countries = snapshot(&memory, 512);
            let mut owner = Owner::new(2, 16, 2);
            let allocations = owner.allocations();
            let mut request = owner.take(&countries, [true, true]);
            let report = run(&mut request, &memory);
            assert_eq!(memory.counts(), [4, 2, 2]);
            let (failed, independent) = if matches!(mutation, Mutation::NameFailure) {
                (report.factions, report.members)
            } else {
                (report.members, report.factions)
            };
            assert!(matches!(failed.result, Outcome::Unavailable { .. }));
            assert_eq!(failed.second, Outcome::Rows { len: 2 });
            // A failed name walk also resets byte metadata, so the original
            // predicate counts both the outcome and name-metadata mismatch.
            assert_eq!(
                failed.mismatches,
                if matches!(mutation, Mutation::NameFailure) {
                    2
                } else {
                    1
                }
            );
            assert!(!matched(failed));
            assert!(matched(independent));
            let lengths = owner.restore(request);
            assert_eq!(
                lengths,
                if matches!(mutation, Mutation::NameFailure) {
                    (0, 0)
                } else {
                    (4, 4)
                }
            );
            assert_eq!(owner.allocations(), allocations);
        }
    }

    #[test]
    fn factions_country_changes_and_failures_remain_subject_to_outer_verification() {
        struct Changed {
            bytes: Vec<u8>,
            changed: Cell<bool>,
            unreadable: bool,
        }
        impl Memory for Changed {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if !(SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                })
                .copy(address, output)
                {
                    return false;
                }
                if self.changed.get() && address == GS + cgame_state::COUNTRIES {
                    if self.unreadable {
                        return false;
                    }
                    output[0] ^= 1;
                }
                true
            }
        }
        for unreadable in [false, true] {
            let memory = Changed {
                bytes: fixture(),
                changed: Cell::new(false),
                unreadable,
            };
            let countries = snapshot(&memory, 512);
            assert_eq!(
                countries.view(&memory).seed_status(),
                engine::country_memory::SeedStatus::Ready
            );
            memory.changed.set(true);
            let mut owner = Owner::new(2, 16, 2);
            let mut request = owner.take(&countries, [true, true]);
            let report = run(&mut request, &memory);
            assert!(matched(report.factions) && matched(report.members));
            owner.restore(request);
            assert_eq!(Arc::strong_count(&countries), 1);
            assert!(
                countries.view(&memory).verify().is_err(),
                "the callback must withhold credit/publication after the shared source verdict"
            );
        }
    }

    #[test]
    fn factions_panic_and_cancellation_return_every_buffer_without_retry_or_credit() {
        for needed in [[true, false], [true, true]] {
            let bytes = fixture();
            let countries = snapshot(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                512,
            );
            let calls = Arc::new(AtomicUsize::new(0));
            let worker_calls = Arc::clone(&calls);
            let pool = acquisition::Pool::start(0, 2, move |request: &mut Request| {
                worker_calls.fetch_add(1, Ordering::Relaxed);
                run(request, &Observed::new(&bytes, Mutation::NamePanic))
            })
            .unwrap();
            let mut owners = [Owner::new(2, 16, 2), Owner::new(2, 16, 2)];
            let allocations = owners.each_ref().map(Owner::allocations);
            let tickets = owners.each_mut().map(|owner| {
                // SAFETY: all synthetic sources and outputs are owned until both completions.
                unsafe { pool.submit(&mut Some(owner.take(&countries, needed))) }.unwrap()
            });
            for (index, ticket) in tickets.into_iter().enumerate() {
                let completion = pool.finish(ticket).unwrap();
                let expected = if index == 0 {
                    acquisition::Failure::Panicked
                } else {
                    acquisition::Failure::Stopped
                };
                assert!(matches!(completion.report, Err(failure) if failure == expected));
                let report = Report::failed(needed, "terminal factions failure");
                assert!(matches!(
                    report.factions.result,
                    Outcome::Unavailable { .. }
                ));
                assert!(matches!(report.members.result, Outcome::Unavailable { .. }));
                assert!(!matched(report.factions) && !matched(report.members));
                assert_eq!(
                    [report.factions.mismatches, report.members.mismatches],
                    needed.map(usize::from)
                );
                assert_eq!(report.ticks, [0; 2]);
                assert_eq!(
                    owners[index].restore(completion.request),
                    if index == 0 { (4, 4) } else { (0, 0) }
                );
                assert_eq!(owners[index].allocations(), allocations[index]);
            }
            assert_eq!(&owners[0].names[..4], &NAME);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(pool.calibration().uncollected, 0);
            pool.shutdown().unwrap();
        }
    }

    #[test]
    fn factions_rejected_dispatch_runs_inline_once_and_routes_the_owned_completion() {
        let countries = Arc::new(Cache::with_capacity(0));
        let pool = acquisition_jobs::Pool::start(0, 1).unwrap();
        let mut batch = acquisition_jobs::Batch::new(&pool);
        let mut owners = [Owner::new(2, 16, 2), Owner::new(2, 16, 2)];
        let allocations = owners.each_ref().map(Owner::allocations);
        let mut pending = owners.each_mut().map(|owner| {
            let mut request = owner.take(&countries, [false, true]);
            // Overflow rejects both roots before the production runner can
            // issue a LiveMemory read, including on its inline fallback path.
            request.factions.game_state = usize::MAX;
            request.members.game_state = usize::MAX;
            Some(acquisition_jobs::Job::Factions(request))
        });
        // SAFETY: overflowed roots cannot reach native memory; all jobs are collected.
        let ticket = unsafe { batch.submit(&mut pending[0]) }.unwrap();
        assert!(matches!(
            unsafe { batch.submit(&mut pending[1]) },
            Err(acquisition::SubmitError::Full)
        ));
        assert!(pending[0].is_none() && pending[1].is_some());
        let accepted = batch.finish(ticket).unwrap();
        let inline = unsafe { batch.run_inline(pending[1].take().unwrap()) };
        for (index, completion) in [accepted, inline].into_iter().enumerate() {
            let acquisition_jobs::Job::Factions(request) = completion.request else {
                panic!("factions request type changed");
            };
            let Ok(acquisition_jobs::Report::Factions(report)) = completion.report else {
                panic!("factions report type changed");
            };
            assert!(matches!(
                report.factions.result,
                Outcome::Unavailable { .. }
            ));
            assert!(matches!(report.members.result, Outcome::Unavailable { .. }));
            assert!(!matched(report.factions) && !matched(report.members));
            assert_eq!(owners[index].restore(request), (0, 0));
            assert_eq!(owners[index].allocations(), allocations[index]);
        }
        assert_eq!(Arc::strong_count(&countries), 1);
        let calibration = pool.calibration();
        let role = calibration
            .roles
            .iter()
            .find(|role| role.role == acquisition_jobs::Role::Factions)
            .unwrap();
        assert_eq!(role.counters.accepted, 1);
        assert_eq!(role.counters.completed, 1);
        assert_eq!(role.counters.rejected, 1);
        assert_eq!(role.counters.inline_completed, 1);
        assert_eq!(calibration.executor.uncollected, 0);
        pool.shutdown().unwrap();
    }
}
