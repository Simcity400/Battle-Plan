use super::advisor_store::{PoolReuse, TokenReuse};
use super::*;
use crate::advisor_pool_changes::{Counts, Pair, Watches};
use engine_characters::advisor_tokens as tokens;
use engine_native::{
    cadvisor,
    ccharacter::{self, manager as ccharacter_manager},
    ccountry,
    cgame_state,
};
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const BASE: usize = 0x0001_4000_0000;
const GAME: usize = 0x1000;
const DETAIL: usize = 0x8000;
const POOLS: [usize; 2] = [0x5000, 0x5400];
const MANAGER: usize = 0x6000;
const BUCKETS: usize = 0xe000;
const DAY: i32 = replay_data::cadence::STAGGER_PERIOD;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    put(bytes, at, &value.to_le_bytes());
}
fn p32(bytes: &mut [u8], at: usize, value: u32) {
    put(bytes, at, &value.to_le_bytes());
}
fn string(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + 32].fill(0);
    put(bytes, at, value);
    p64(bytes, at + 16, value.len() as u64);
    p64(bytes, at + 24, 15);
}

fn advisor(country: usize, ordinal: usize) -> (usize, usize) {
    let advisor = DETAIL + country * 0x1000 + ordinal * 0x200;
    (advisor, advisor + 0x100)
}

fn world() -> Vec<u8> {
    let mut bytes = vec![0; 0x10000];
    for (offset, data) in [
        (cgame_state::COUNTRIES, 0x2000_u64),
        (cgame_state::COUNTRY_LINKS, 0x2100),
    ] {
        p64(&mut bytes, GAME + offset, data);
        p32(&mut bytes, GAME + offset + 8, 3);
        p32(&mut bytes, GAME + offset + 12, 3);
    }
    for (index, country) in [0x4000_usize, 0x4800].into_iter().enumerate() {
        let slot = index + 1;
        p64(&mut bytes, 0x2000 + slot * 8, country as u64);
        p32(&mut bytes, 0x2100 + slot * 4, slot as u32);
        p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA.0) as u64);
        p32(&mut bytes, country + ccountry::ID, slot as u32);
        let pool = POOLS[index];
        p64(&mut bytes, country + ccountry::CHARACTERS, pool as u64);
        p64(
            &mut bytes,
            pool,
            (BASE + engine_native::ccountry::rosters::VTABLE_RVA.0) as u64,
        );
        p64(
            &mut bytes,
            pool + engine_native::ccountry::rosters::OWNER,
            country as u64,
        );
        let array = pool + 0x200;
        let header = pool + engine_native_politics::ccountry_characters::AVAILABLE_ADVISORS;
        p64(&mut bytes, header, array as u64);
        p32(&mut bytes, header + 8, 2);
        p32(&mut bytes, header + 12, 2);
        for ordinal in 0..2 {
            let (role, character) = advisor(index, ordinal);
            p64(&mut bytes, array + ordinal * 8, role as u64);
            p64(&mut bytes, role, (BASE + cadvisor::VTABLE_RVA.0) as u64);
            p64(&mut bytes, role + cadvisor::CHARACTER, character as u64);
            string(
                &mut bytes,
                role + cadvisor::IDEA_TOKEN,
                format!("idea_{index}_{ordinal}").as_bytes(),
            );
            p64(
                &mut bytes,
                character,
                (BASE + ccharacter::VTABLE_RVA.0) as u64,
            );
            p32(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE,
            );
            p32(
                &mut bytes,
                character + ccharacter::ID,
                100 + (index * 2 + ordinal) as u32,
            );
        }
    }
    p64(
        &mut bytes,
        GAME + cgame_state::CHARACTER_MANAGER,
        MANAGER as u64,
    );
    p64(
        &mut bytes,
        MANAGER,
        (BASE + ccharacter_manager::VTABLE_RVA.0) as u64,
    );
    let table = MANAGER + tokens::TABLE;
    p64(&mut bytes, table + tokens::BUCKETS, BUCKETS as u64);
    p32(&mut bytes, table + tokens::COUNT, 3);
    p32(&mut bytes, table + tokens::MASK, 7);
    bytes[table + tokens::MAXIMUM_PROBE] = 1;
    for (index, key) in [(1, b"b_idea".as_slice()), (4, b"c_idea"), (8, b"a_idea")] {
        let bucket = BUCKETS + index * tokens::STRIDE;
        bytes[bucket + tokens::DISTANCE] = 1;
        string(&mut bytes, bucket + tokens::KEY, key);
    }
    bytes[BUCKETS + 9 * tokens::STRIDE + tokens::DISTANCE] = 0xff;
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
    available: (Outcome, Vec<native::Row>, Vec<u8>),
    tokens: (Outcome, Vec<tokens::Row>, Vec<u8>),
    pools: Counts,
    table: Counts,
    detail_reads: usize,
}

fn hour(bytes: &[u8], store: &mut AdvisorStore, changes: &Pair<'_>) -> Hour {
    hour_at(bytes, store, changes, 0, &|_| true)
}

fn hour_at(
    bytes: &[u8],
    store: &mut AdvisorStore,
    changes: &Pair<'_>,
    at: i32,
    select: &dyn Fn(u32) -> bool,
) -> Hour {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let (mut rows, mut text) = (Vec::new(), Vec::new());
    let mut carry = PoolReuse::begin(store, changes, at);
    let walk = native::walk_available_carried(
        &memory, BASE, GAME, &mut rows, &mut text, &mut carry, select,
    );
    carry.finish(matches!(walk.outcome, Outcome::Rows { .. }), 0);
    rows.truncate(walk.outcome.len());
    text.truncate(walk.bytes_len);
    let (mut keys, mut key_bytes) = (Vec::new(), Vec::new());
    let mut carry = TokenReuse::begin(store, changes);
    let token_walk =
        tokens::walk_carried(&memory, BASE, GAME, &mut keys, &mut key_bytes, &mut carry);
    carry.finish(matches!(token_walk.outcome, Outcome::Rows { .. }));
    keys.truncate(token_walk.outcome.len());
    key_bytes.truncate(token_walk.bytes_len);
    let (pools, table) = store.counts();
    Hour {
        available: (walk.outcome, rows, text),
        tokens: (token_walk.outcome, keys, key_bytes),
        pools,
        table,
        detail_reads: memory
            .calls
            .into_inner()
            .into_iter()
            .filter(|&address| address >= DETAIL)
            .count(),
    }
}

struct Domains {
    pools: Watches,
    characters: crate::character_changes::Watches,
}

impl Domains {
    fn new() -> Self {
        Self {
            pools: Watches::enabled(16),
            characters: crate::character_changes::Watches::enabled(16),
        }
    }
    fn pair(&self) -> Pair<'_> {
        Pair {
            pools: &self.pools,
            characters: &self.characters,
        }
    }
    fn boundary(&self, invalidate: bool) {
        self.pools.boundary(invalidate);
        self.characters.boundary(invalidate);
    }
}

fn first_hour(bytes: &[u8], store: &mut AdvisorStore, domains: &Domains) -> Hour {
    let first = hour(bytes, store, &domains.pair());
    assert_eq!(first.available.0, Outcome::Rows { len: 4 });
    assert_eq!(first.tokens.0, Outcome::Rows { len: 3 });
    assert_eq!((first.pools.read, first.pools.reused), (2, 0));
    assert_eq!((first.table.read, first.table.reused), (1, 0));
    assert_ne!(first.detail_reads, 0);
    domains.boundary(false);
    first
}

#[test]
fn quiet_hours_read_no_advisor_or_bucket_and_publish_identical_rows() {
    let bytes = world();
    let domains = Domains::new();
    let mut store = AdvisorStore::new();
    let first = first_hour(&bytes, &mut store, &domains);
    for _ in 0..3 {
        let quiet = hour(&bytes, &mut store, &domains.pair());
        assert_eq!(quiet.available, first.available);
        assert_eq!(quiet.tokens, first.tokens);
        assert_eq!((quiet.pools.read, quiet.pools.reused), (0, 2));
        assert_eq!((quiet.table.read, quiet.table.reused), (0, 1));
        assert_eq!(quiet.detail_reads, 0);
        domains.boundary(false);
    }
}

#[test]
fn a_hooked_pool_write_rereads_only_its_pool() {
    let mut bytes = world();
    let domains = Domains::new();
    let mut store = AdvisorStore::new();
    let first = first_hour(&bytes, &mut store, &domains);
    let notification = domains.pools.notify(Some(POOLS[1]), 3);
    p32(
        &mut bytes,
        POOLS[1] + engine_native_politics::ccountry_characters::AVAILABLE_ADVISORS + 12,
        1,
    );
    drop(notification);
    domains.boundary(false);
    let changed = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((changed.pools.read, changed.pools.reused), (1, 1));
    assert_eq!(changed.available.0, Outcome::Rows { len: 3 });
    assert_eq!(changed.available.1[..2], first.available.1[..2]);
    assert_eq!((changed.table.read, changed.table.reused), (0, 1));
}

#[test]
fn a_character_write_rereads_only_the_pool_listing_it() {
    let mut bytes = world();
    let domains = Domains::new();
    let mut store = AdvisorStore::new();
    first_hour(&bytes, &mut store, &domains);
    let (role, character) = advisor(0, 0);
    let notification = domains.characters.notify(Some(character), 0);
    string(&mut bytes, role + cadvisor::IDEA_TOKEN, b"changed");
    drop(notification);
    domains.boundary(false);
    let changed = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((changed.pools.read, changed.pools.reused), (1, 1));
    let (_, rows, text) = &changed.available;
    assert_eq!(&text[rows[0].idea_start..][..rows[0].idea_len], b"changed");
    domains.boundary(false);
    let quiet = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((quiet.pools.read, quiet.pools.reused), (0, 2));
    assert_eq!(quiet.available, changed.available);
}

#[test]
fn a_hooked_token_write_or_an_invalidation_rereads_the_table() {
    let mut bytes = world();
    let domains = Domains::new();
    let mut store = AdvisorStore::new();
    first_hour(&bytes, &mut store, &domains);
    let notification = domains.pools.notify(Some(MANAGER), 8);
    string(
        &mut bytes,
        BUCKETS + 4 * tokens::STRIDE + tokens::KEY,
        b"d_idea",
    );
    drop(notification);
    domains.boundary(false);
    let changed = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((changed.table.read, changed.table.reused), (1, 0));
    assert_eq!((changed.pools.read, changed.pools.reused), (0, 2));
    assert!(changed.tokens.2.windows(6).any(|key| key == b"d_idea"));
    domains.boundary(true);
    let reloaded = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((reloaded.pools.read, reloaded.table.read), (2, 1));
    domains.boundary(false);
    drop(domains.pools.notify(None, 5));
    let unnamed = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((unnamed.pools.read, unnamed.table.read), (2, 1));
    domains.boundary(false);
    store.forget();
    let forgotten = hour(&bytes, &mut store, &domains.pair());
    assert_eq!((forgotten.pools.read, forgotten.table.read), (2, 1));
}

#[test]
fn a_pool_read_at_its_slice_is_carried_a_day_later() {
    let bytes = world();
    let domains = Domains::new();
    let mut store = AdvisorStore::new();
    let slice = |at: i32| move |country: u32| country % DAY as u32 == at as u32;
    let mut day = Vec::new();
    for at in 0..DAY {
        let walked = hour_at(&bytes, &mut store, &domains.pair(), at, &slice(at));
        let read = usize::from(at == 1 || at == 2);
        assert_eq!(walked.available.0, Outcome::Rows { len: 2 * read });
        assert_eq!((walked.pools.read, walked.pools.reused), (read as u64, 0));
        day.push(walked.available);
        domains.boundary(false);
    }
    for at in 0..DAY {
        let next = hour_at(&bytes, &mut store, &domains.pair(), at + DAY, &slice(at));
        let read = u64::from(at == 1 || at == 2);
        assert_eq!((next.pools.read, next.pools.reused), (0, read), "hour {at}");
        assert_eq!(next.available, day[at as usize]);
        domains.boundary(false);
    }
}
