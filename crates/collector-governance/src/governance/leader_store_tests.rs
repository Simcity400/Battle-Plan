use super::*;
use crate::party_leader_changes::{Counts, Pair, Watches};
use engine_native::{ccharacter, ccountry, cideology_group, memory::SliceMemory};
use engine_native_politics::{ccountry_leader, cpolitical_party::LeaderRow};
use std::cell::RefCell;

const REGION: usize = 0x2000;
const COUNTRIES: usize = 2;

fn word(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn objects(index: usize) -> (usize, usize, usize) {
    let at = index * REGION;
    (at + 0x1500, at + 0x1800, at + 0x1a00)
}

fn world() -> Vec<u8> {
    let mut bytes = vec![0_u8; COUNTRIES * REGION];
    for index in 0..COUNTRIES {
        let at = index * REGION;
        let b = &mut bytes;
        word(b, at + 0x100, ccountry::VTABLE_RVA.0 as u64);
        word(
            b,
            at + 0x100 + ccountry::POLITICAL_STATUS,
            (at + 0x1200) as u64,
        );
        word(b, at + 0x1200, cpolitical_status::VTABLE_RVA.0 as u64);
        word(
            b,
            at + 0x1200 + cpolitical_status::COUNTRY,
            (at + 0x100) as u64,
        );
        word(
            b,
            at + 0x1200 + cpolitical_status::PARTIES,
            (at + 0x1400) as u64,
        );
        word(
            b,
            at + 0x1200 + cpolitical_status::PARTIES + 8,
            (1 << 32) | 1,
        );
        let (party, leader, character) = objects(index);
        word(b, at + 0x1400, party as u64);
        word(b, party, cpolitical_party::VTABLE_RVA.0 as u64);
        word(b, party + cpolitical_party::ID, 7);
        word(
            b,
            party + cpolitical_party::IDEOLOGY_GROUP,
            (at + 0x1600) as u64,
        );
        word(b, at + 0x1600, cideology_group::VTABLE_RVA.0 as u64);
        word(b, at + 0x1600 + cideology_group::ID, 7);
        word(
            b,
            party + cpolitical_party::COUNTRY_LEADERS,
            (at + 0x1700) as u64,
        );
        word(
            b,
            party + cpolitical_party::COUNTRY_LEADERS + 8,
            (1 << 32) | 1,
        );
        word(b, at + 0x1700, leader as u64);
        word(b, leader, ccountry_leader::VTABLE_RVA.0 as u64);
        word(b, leader + ccountry_leader::CHARACTER, character as u64);
        word(b, leader + ccountry_leader::LEADER_ID, 12);
        word(b, character, ccharacter::VTABLE_RVA.0 as u64);
        word(
            b,
            character + ccharacter::TYPE,
            ((91 + index as u64) << 32) | u64::from(ccharacter::REFERENCE_TYPE),
        );
        let (head, node) = (at + 0x1c00, at + 0x1c40);
        word(b, character + ccharacter::LEADER_MAP_HEAD, head as u64);
        word(b, character + ccharacter::LEADER_MAP_SIZE, 1);
        for offset in [0, 8, 16] {
            word(b, head + offset, node as u64);
            word(b, node + offset, head as u64);
        }
        word(b, head + 0x18, 1 << 8);
        word(b, node + 0x20, u64::from((-7_i32) as u32));
        word(b, node + 0x28, leader as u64);
    }
    bytes
}

struct Calls<'a> {
    bytes: &'a [u8],
    calls: RefCell<Vec<usize>>,
}
impl Memory for Calls<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push(address);
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

struct Hour {
    rows: Vec<LeaderRow>,
    counts: Counts,
    detail_reads: usize,
}

struct Domains {
    parties: Watches,
    characters: crate::character_changes::Watches,
}

impl Domains {
    fn new() -> Self {
        Self {
            parties: Watches::enabled(16),
            characters: crate::character_changes::Watches::enabled(16),
        }
    }
    fn boundary(&self, invalidate: bool) {
        self.parties.boundary(invalidate);
        self.characters.boundary(invalidate);
    }
}

fn hour(bytes: &[u8], store: &mut LeaderStore, domains: &Domains) -> Hour {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let pair = Pair {
        parties: &domains.parties,
        characters: &domains.characters,
    };
    let mut reuse = LeaderReuse::begin(store, &pair);
    let mut rows = Vec::new();
    let mut complete = true;
    for index in 0..COUNTRIES {
        let reader = cpolitical_status::Reader::new(&memory, 0, index * REGION + 0x100).unwrap();
        let first = rows.len();
        let outcome = reader.party_leaders_carried(
            index as u32 + 1,
            engine_native::walk::Buffer::Growing(&mut rows).suffix(first),
            &mut reuse,
        );
        complete &= matches!(outcome, Outcome::Rows { len: 1 });
        rows.truncate(first + outcome.len());
    }
    reuse.finish(complete);
    assert!(complete);
    Hour {
        rows,
        counts: store.counts(),
        detail_reads: memory
            .calls
            .into_inner()
            .into_iter()
            .filter(|&address| address % REGION >= 0x1500)
            .count(),
    }
}

fn first_hour(bytes: &[u8], store: &mut LeaderStore, domains: &Domains) -> Hour {
    let first = hour(bytes, store, domains);
    assert_eq!((first.counts.read, first.counts.reused), (2, 0));
    assert_eq!(
        first
            .rows
            .iter()
            .map(|row| (row.country, row.character, row.role, row.leader_id))
            .collect::<Vec<_>>(),
        [(1, Some(91), Some(-7), 12), (2, Some(92), Some(-7), 12)]
    );
    assert_ne!(first.detail_reads, 0);
    domains.boundary(false);
    first
}

#[test]
fn quiet_hours_read_no_party_or_leader_and_publish_identical_rows() {
    let bytes = world();
    let domains = Domains::new();
    let mut store = LeaderStore::new();
    let first = first_hour(&bytes, &mut store, &domains);
    for _ in 0..3 {
        let quiet = hour(&bytes, &mut store, &domains);
        assert_eq!(quiet.rows, first.rows);
        assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 2));
        assert_eq!(quiet.detail_reads, 0);
        domains.boundary(false);
    }
}

#[test]
fn a_hooked_party_or_character_write_rereads_only_its_country() {
    let mut bytes = world();
    let domains = Domains::new();
    let mut store = LeaderStore::new();
    first_hour(&bytes, &mut store, &domains);
    let (party, leader, _) = objects(1);
    let notification = domains.parties.notify(party, 1);
    word(&mut bytes, leader + ccountry_leader::LEADER_ID, 13);
    drop(notification);
    domains.boundary(false);
    let changed = hour(&bytes, &mut store, &domains);
    assert_eq!((changed.counts.read, changed.counts.reused), (1, 1));
    assert_eq!(changed.rows[1].leader_id, 13);
    domains.boundary(false);
    let (_, _, character) = objects(0);
    let notification = domains.characters.notify(Some(character), 2);
    word(&mut bytes, 0x1c40 + 0x20, u64::from((-8_i32) as u32));
    drop(notification);
    domains.boundary(false);
    let changed = hour(&bytes, &mut store, &domains);
    assert_eq!((changed.counts.read, changed.counts.reused), (1, 1));
    assert_eq!(changed.rows[0].role, Some(-8));
    domains.boundary(false);
    let quiet = hour(&bytes, &mut store, &domains);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 2));
    assert_eq!(quiet.rows, changed.rows);
}

#[test]
fn an_invalidating_hour_or_a_forgotten_store_rereads_every_country() {
    let bytes = world();
    let domains = Domains::new();
    let mut store = LeaderStore::new();
    let first = first_hour(&bytes, &mut store, &domains);
    domains.boundary(true);
    let reloaded = hour(&bytes, &mut store, &domains);
    assert_eq!((reloaded.counts.read, reloaded.counts.reused), (2, 0));
    assert_eq!(reloaded.rows, first.rows);
    domains.boundary(false);
    store.forget();
    let forgotten = hour(&bytes, &mut store, &domains);
    assert_eq!((forgotten.counts.read, forgotten.counts.reused), (2, 0));
}
