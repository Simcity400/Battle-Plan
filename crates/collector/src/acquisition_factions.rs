//! Factions and ordered membership acquired once within the country lease.

use crate::{
    acquisition_gather::{Report as GatherReport, Request as GatherRequest},
    sampling::qpc,
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
    pub(crate) fn restore(
        self,
        factions: &mut Vec<cfaction::Row>,
        names: &mut Vec<u8>,
        members: &mut Vec<cfaction_system::MemberRow>,
    ) -> (usize, usize) {
        let payload = self.factions.payload;
        let lengths = (payload.bytes_len, payload.required_bytes);
        payload.restore(factions, names);
        self.members.restore(members);
        lengths
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub factions: GatherReport,
    pub members: GatherReport,
    pub ticks: [u64; 2],
}

impl Report {
    pub(crate) fn failed(reason: &'static str) -> Self {
        Self {
            factions: GatherReport {
                result: Outcome::Unavailable { reason },
            },
            members: GatherReport {
                result: Outcome::Unavailable { reason },
            },
            ticks: [0; 2],
        }
    }
}

pub(crate) fn run(request: &mut Request, memory: &impl Memory) -> Report {
    let _factions_reads = crate::read_accounting::table(replay::rows::TableId::Factions);
    let cached = request.countries.view(memory);
    let names = &mut request.factions;
    let (base, game_state) = (names.base, names.game_state);
    let started = qpc();
    let factions = names.run(|payload| payload.walk(&cached, base, game_state));
    let faction_ticks = qpc().saturating_sub(started).max(0) as u64;
    let _members_reads = crate::read_accounting::table(replay::rows::TableId::FactionMembers);
    let members = &mut request.members;
    let (base, game_state) = (members.base, members.game_state);
    let started = qpc();
    let members =
        members.run(|rows| cfaction_system::walk_members(&cached, base, game_state, rows));
    Report {
        factions,
        members,
        ticks: [faction_ticks, qpc().saturating_sub(started).max(0) as u64],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acquisition;
    use engine::{ccountry, cdiplomacy_status, cgame_state, memory::SliceMemory, msvc_string};
    use std::cell::RefCell;

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

    fn snapshot(bytes: &[u8], cells: usize) -> Arc<Cache> {
        let mut storage = Arc::new(Cache::with_capacity(cells));
        Cache::prepare_shared(&mut storage, &SliceMemory { base: 0, bytes }, 0, GS).unwrap()
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
            self.calls.borrow_mut().push((address, output.len()));
            if self.fault == Some(address) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    fn take(
        countries: &Arc<Cache>,
        rows: &mut Vec<cfaction::Row>,
        names: &mut Vec<u8>,
        members: &mut Vec<cfaction_system::MemberRow>,
    ) -> Request {
        Request {
            countries: Arc::clone(countries),
            factions: GatherRequest {
                base: 0,
                game_state: GS,
                payload: Names::take(rows, names),
            },
            members: GatherRequest::take(0, GS, members),
        }
    }

    #[test]
    fn single_factions_acquisition_preserves_rows_arenas_capacity_and_read_trace() {
        let bytes = fixture();
        for cells in [0, 512] {
            let countries = snapshot(&bytes, cells);
            for (capacity, name_capacity, member_capacity) in
                [(2, 16, 2), (0, 0, 0), (1, 16, 1), (2, 0, 2)]
            {
                let reference = Traced::new(&bytes);
                let cached = countries.view(&reference);
                let mut expected_rows = vec![cfaction::Row::default(); capacity];
                let mut expected_names = vec![0; name_capacity];
                let mut expected_members =
                    vec![cfaction_system::MemberRow::default(); member_capacity];
                let names = cfaction::walk(&cached, 0, GS, &mut expected_rows, &mut expected_names);
                let members = cfaction_system::walk_members(&cached, 0, GS, &mut expected_members);
                let actual = Traced::new(&bytes);
                let mut rows = vec![cfaction::Row::default(); capacity];
                let mut bytes_out = vec![0; name_capacity];
                let mut output = vec![cfaction_system::MemberRow::default(); member_capacity];
                let pointers = (rows.as_ptr(), bytes_out.as_ptr(), output.as_ptr());
                let mut request = take(&countries, &mut rows, &mut bytes_out, &mut output);
                let report = run(&mut request, &actual);
                let lengths = request.restore(&mut rows, &mut bytes_out, &mut output);
                assert_eq!(
                    (report.factions.result, report.members.result),
                    (names.outcome, members)
                );
                assert_eq!(lengths, (names.bytes_len, names.required_bytes));
                assert_eq!(rows, expected_rows);
                assert_eq!(bytes_out, expected_names);
                assert_eq!(output, expected_members);
                assert_eq!(
                    (rows.as_ptr(), bytes_out.as_ptr(), output.as_ptr()),
                    pointers
                );
                assert_eq!(*actual.calls.borrow(), *reference.calls.borrow());
                assert_eq!(
                    actual
                        .calls
                        .borrow()
                        .iter()
                        .filter(|(at, _)| *at == GS + cgame_state::FACTION_SYSTEM)
                        .count(),
                    2
                );
                if capacity == 2 && name_capacity == 16 && member_capacity == 2 {
                    assert_eq!(report.factions.result, Outcome::Rows { len: 2 });
                    assert_eq!(report.members.result, Outcome::Rows { len: 2 });
                    assert_eq!(&bytes_out[..4], &NAME);
                    assert_eq!(
                        output.iter().map(|row| row.country).collect::<Vec<_>>(),
                        [2, 7]
                    );
                }
            }
        }
    }

    #[test]
    fn name_failure_does_not_repeat_factions_or_suppress_members() {
        let bytes = fixture();
        let countries = snapshot(&bytes, 512);
        let mut memory = Traced::new(&bytes);
        memory.fault = Some(FIRST + cfaction::NAME);
        let mut rows = vec![cfaction::Row::default(); 2];
        let mut names = vec![0; 16];
        let mut members = vec![cfaction_system::MemberRow::default(); 2];
        let mut request = take(&countries, &mut rows, &mut names, &mut members);
        let report = run(&mut request, &memory);
        assert!(matches!(
            report.factions.result,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(report.members.result, Outcome::Rows { len: 2 });
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == FIRST + cfaction::NAME)
                .count(),
            1
        );
    }

    #[test]
    fn pool_returns_all_single_faction_buffers() {
        for workers in [0, 2] {
            let bytes = fixture();
            let countries = snapshot(&bytes, 512);
            let pool = acquisition::Pool::start(workers, 1, move |request: &mut Request| {
                run(
                    request,
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    },
                )
            })
            .unwrap();
            let mut rows = vec![cfaction::Row::default(); 2];
            let mut names = vec![0; 16];
            let mut members = vec![cfaction_system::MemberRow::default(); 2];
            let pointers = (rows.as_ptr(), names.as_ptr(), members.as_ptr());
            let mut pending = Some(take(&countries, &mut rows, &mut names, &mut members));
            // SAFETY: synthetic memory and every output are owned through collection.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completion = pool.finish(ticket).unwrap();
            let report = completion.report.unwrap();
            assert_eq!(report.factions.result, Outcome::Rows { len: 2 });
            assert_eq!(report.members.result, Outcome::Rows { len: 2 });
            assert_eq!(
                completion
                    .request
                    .restore(&mut rows, &mut names, &mut members),
                (4, 4)
            );
            assert_eq!((rows.as_ptr(), names.as_ptr(), members.as_ptr()), pointers);
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }
}
