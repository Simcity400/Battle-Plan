use super::*;
use crate::party_changes::{Counts, Watches};
use engine_native::ccountry;
use engine_native_parts::{cidea, cideology_group};
use engine_native_memory::{memory::SliceMemory, msvc_string};
use engine_native_politics::{
    cpolitical_party,
    cpolitical_status::{FreshIdeas, FreshParties, IdeaRow},
};
use std::cell::RefCell;

const REGION: usize = 0x2000;
const COUNTRIES: usize = 2;

fn word(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn string(bytes: &mut [u8], at: usize, text: &[u8]) {
    assert!(text.len() <= 15);
    bytes[at..at + 16].fill(0);
    bytes[at..at + text.len()].copy_from_slice(text);
    word(bytes, at + msvc_string::LENGTH, text.len() as u64);
    word(
        bytes,
        at + msvc_string::CAPACITY,
        msvc_string::INLINE_CAPACITY,
    );
}

fn status(index: usize) -> usize {
    index * REGION + 0x200
}
fn party(index: usize, party: usize) -> usize {
    index * REGION + 0x500 + party * 0x100
}
fn idea(index: usize, idea: usize) -> usize {
    index * REGION + 0x800 + idea * 0x100
}

fn set_popularity(bytes: &mut [u8], country: usize, which: usize, value: i64) {
    word(
        bytes,
        party(country, which) + cpolitical_party::POPULARITY,
        value as u64,
    );
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
            status(index) as u64,
        );
        word(b, status(index), engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0 as u64);
        word(
            b,
            status(index) + engine_native_parts::cpolitical_status_layout::COUNTRY,
            (at + 0x100) as u64,
        );
        word(
            b,
            status(index) + cpolitical_status::PARTIES,
            (at + 0x400) as u64,
        );
        word(
            b,
            status(index) + cpolitical_status::PARTIES + 8,
            (2 << 32) | 2,
        );
        word(
            b,
            status(index) + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
            party(index, 0) as u64,
        );
        word(
            b,
            status(index) + cpolitical_status::IDEAS,
            (at + 0x480) as u64,
        );
        word(
            b,
            status(index) + cpolitical_status::IDEAS + 8,
            (2 << 32) | 2,
        );
        for (which, id) in [(0, 3_u64), (1, 7)] {
            let party = party(index, which);
            let ideology = at + 0x700 + which * 0x80;
            word(b, at + 0x400 + which * 8, party as u64);
            word(b, party, engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0 as u64);
            word(b, party + engine_native_parts::cpolitical_party_layout::ID, id);
            word(b, party + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, ideology as u64);
            word(b, ideology, cideology_group::VTABLE_RVA.0 as u64);
            word(b, ideology + cideology_group::ID, id);
            string(
                b,
                ideology + cideology_group::TOKEN,
                if which == 0 {
                    b"democratic"
                } else {
                    b"fascism"
                },
            );
            word(b, party + cpolitical_party::TAG, 40 + index as u64);
            string(b, party + cpolitical_party::NAME, b"PARTY");
            string(b, party + cpolitical_party::LONG_NAME, b"PARTY_LONG");
            b[party + cpolitical_party::NAME_DEFAULT] = 0;
            set_popularity(b, index, which, 50_000 * (which as i64 + 1));
            let idea = idea(index, which);
            word(b, at + 0x480 + which * 8, idea as u64);
            word(b, idea, cidea::VTABLE_RVA.0 as u64);
            string(
                b,
                idea + cidea::TOKEN,
                if which == 0 { b"idea_a" } else { b"idea_b" },
            );
        }
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
    parties: Vec<(u32, cpolitical_party::Row, Vec<u8>)>,
    ideas: Vec<(u32, IdeaRow, Vec<u8>)>,
    counts: [Counts; 2],
    content_reads: usize,
}

fn walk(bytes: &[u8], store: &mut PartyStore, watches: &Watches) -> Hour {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let mut reuse = PoliticsReuse::begin(store, watches);
    let mut hour = Hour {
        parties: Vec::new(),
        ideas: Vec::new(),
        counts: [Counts::default(); 2],
        content_reads: 0,
    };
    let (mut complete_parties, mut complete_ideas) = (true, true);
    for index in 0..COUNTRIES {
        let country = 10 + index as u32;
        let reader = cpolitical_status::Reader::new(&memory, 0, index * REGION + 0x100).unwrap();
        let (mut rows, mut text) = (Vec::new(), Vec::new());
        let walked = reader.parties_carried(
            country,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            0,
            engine_native_memory::walk::Buffer::Growing(&mut text),
            0,
            &mut reuse,
        );
        complete_parties &= matches!(walked.outcome, Outcome::Rows { .. });
        for row in &rows[..walked.outcome.len()] {
            let mut names = text[row.token_start..][..row.token_len].to_vec();
            names.extend_from_slice(&text[row.name_start..][..row.name_len]);
            names.extend_from_slice(&text[row.long_name_start..][..row.long_name_len]);
            hour.parties.push((country, *row, names));
        }
        let (mut rows, mut text) = (Vec::new(), Vec::new());
        let walked = reader.ideas_carried(
            country,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            engine_native_memory::walk::Buffer::Growing(&mut text),
            &mut reuse,
        );
        complete_ideas &= matches!(walked.outcome, Outcome::Rows { .. });
        for row in &rows[..walked.outcome.len()] {
            let token = text[row.token_start..][..row.token_len].to_vec();
            hour.ideas.push((country, *row, token));
        }
    }
    reuse.finish(complete_parties, complete_ideas);
    hour.counts = store.counts();
    hour.content_reads = memory
        .calls
        .into_inner()
        .into_iter()
        .filter(|&address| {
            let offset = address % REGION;
            (0x500..0xa00).contains(&offset)
                && offset != 0x500 + cpolitical_party::POPULARITY
                && offset != 0x600 + cpolitical_party::POPULARITY
        })
        .count();
    hour
}

fn fresh(bytes: &[u8]) -> Hour {
    let memory = SliceMemory { base: 0, bytes };
    let mut hour = Hour {
        parties: Vec::new(),
        ideas: Vec::new(),
        counts: [Counts::default(); 2],
        content_reads: 0,
    };
    for index in 0..COUNTRIES {
        let country = 10 + index as u32;
        let reader = cpolitical_status::Reader::new(&memory, 0, index * REGION + 0x100).unwrap();
        let (mut rows, mut text) = (Vec::new(), Vec::new());
        let walked = reader.parties_carried(
            country,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            0,
            engine_native_memory::walk::Buffer::Growing(&mut text),
            0,
            &mut FreshParties,
        );
        for row in &rows[..walked.outcome.len()] {
            let mut names = text[row.token_start..][..row.token_len].to_vec();
            names.extend_from_slice(&text[row.name_start..][..row.name_len]);
            names.extend_from_slice(&text[row.long_name_start..][..row.long_name_len]);
            hour.parties.push((country, *row, names));
        }
        let (mut rows, mut text) = (Vec::new(), Vec::new());
        let walked = reader.ideas_carried(
            country,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            engine_native_memory::walk::Buffer::Growing(&mut text),
            &mut FreshIdeas,
        );
        for row in &rows[..walked.outcome.len()] {
            let token = text[row.token_start..][..row.token_len].to_vec();
            hour.ideas.push((country, *row, token));
        }
    }
    hour
}

fn assert_fresh(hour: &Hour, bytes: &[u8]) {
    let fresh = fresh(bytes);
    assert_eq!(hour.parties, fresh.parties);
    assert_eq!(hour.ideas, fresh.ideas);
}

fn first_hour(bytes: &[u8], store: &mut PartyStore, watches: &Watches) -> Hour {
    let first = walk(bytes, store, watches);
    assert_eq!(first.parties.len(), 4);
    assert_eq!(first.ideas.len(), 4);
    assert_eq!(
        first.counts.map(|counts| (counts.read, counts.reused)),
        [(2, 0), (2, 0)]
    );
    assert_ne!(first.content_reads, 0);
    assert_fresh(&first, bytes);
    watches.boundary(false);
    first
}

#[test]
fn quiet_hours_read_no_party_content_and_publish_fresh_rows() {
    let mut bytes = world();
    let watches = Watches::enabled(64);
    let mut store = PartyStore::new();
    first_hour(&bytes, &mut store, &watches);
    for hour in 0..3_i64 {
        set_popularity(&mut bytes, 1, 1, 70_000 + hour);
        word(
            &mut bytes,
            status(0) + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
            party(0, (hour % 2) as usize) as u64,
        );
        let quiet = walk(&bytes, &mut store, &watches);
        assert_eq!(
            quiet
                .counts
                .map(|counts| (counts.read, counts.reused, counts.reused_rows)),
            [(0, 2, 4), (0, 2, 4)]
        );
        assert_eq!(quiet.content_reads, 0);
        assert_fresh(&quiet, &bytes);
        assert_eq!(quiet.parties[3].1.popularity, 70_000 + hour);
        watches.boundary(false);
    }
}

#[test]
fn a_hooked_party_write_rereads_only_its_country() {
    let mut bytes = world();
    let watches = Watches::enabled(64);
    let mut store = PartyStore::new();
    first_hour(&bytes, &mut store, &watches);
    let notification = watches.notify(party(1, 1), 0);
    string(&mut bytes, party(1, 1) + cpolitical_party::NAME, b"RENAMED");
    bytes[party(1, 1) + cpolitical_party::NAME_DEFAULT] = 0;
    drop(notification);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!(
        hour.counts[0].read, 1,
        "only the written party's country is read"
    );
    assert_eq!(hour.counts[0].reused, 1);
    assert_fresh(&hour, &bytes);
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches);
    assert_eq!((quiet.counts[0].read, quiet.counts[0].reused), (0, 2));
    assert_fresh(&quiet, &bytes);
}

#[test]
fn a_changed_party_or_idea_list_rereads_its_country() {
    let mut bytes = world();
    let watches = Watches::enabled(64);
    let mut store = PartyStore::new();
    first_hour(&bytes, &mut store, &watches);
    for (array, first, second) in [
        (0x480, idea(0, 0), idea(0, 1)),
        (REGION + 0x400, party(1, 0), party(1, 1)),
    ] {
        word(&mut bytes, array, second as u64);
        word(&mut bytes, array + 8, first as u64);
    }
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!(
        hour.counts.map(|counts| (counts.read, counts.reused)),
        [(1, 1), (1, 1)]
    );
    assert_fresh(&hour, &bytes);
}

#[test]
fn an_invalidating_hour_rereads_every_country() {
    let bytes = world();
    let watches = Watches::enabled(64);
    let mut store = PartyStore::new();
    first_hour(&bytes, &mut store, &watches);
    watches.boundary(true);
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!(
        hour.counts.map(|counts| (counts.read, counts.reused)),
        [(2, 0), (2, 0)]
    );
    assert_fresh(&hour, &bytes);
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&bytes, &mut store, &watches);
    assert_eq!(
        forgotten.counts.map(|counts| (counts.read, counts.reused)),
        [(2, 0), (2, 0)]
    );
}

#[test]
fn an_unobserved_write_is_carried_stale() {
    let mut bytes = world();
    let watches = Watches::enabled(64);
    let mut store = PartyStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    string(&mut bytes, party(0, 0) + cpolitical_party::NAME, b"UNSEEN");
    let stale = walk(&bytes, &mut store, &watches);
    assert_eq!(stale.counts[0].reused, 2);
    assert_eq!(stale.parties, first.parties);
}

#[test]
fn a_failed_walk_holds_nothing() {
    let mut bytes = world();
    let watches = Watches::enabled(64);
    let mut store = PartyStore::new();
    first_hour(&bytes, &mut store, &watches);
    let mut broken = bytes.clone();
    word(
        &mut broken,
        status(1) + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
        0x1234,
    );
    let failed = walk(&broken, &mut store, &watches);
    assert_eq!(failed.parties.len(), 2);
    string(&mut bytes, party(0, 0) + cpolitical_party::NAME, b"UNSEEN");
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!(
        hour.counts.map(|counts| (counts.read, counts.reused)),
        [(2, 0), (0, 2)]
    );
    assert_fresh(&hour, &bytes);
}
