//! A quiet politics hour ([`super::politics_hour`]): it publishes what a walk
//! of the same memory publishes while reading no country's politics; a
//! hooked structure write, a moved party-leader certificate or a new country
//! walks; a popularity write gathers only popularity; a change no hook saw is
//! caught by the sampled slice or verify, poisons and walks; and the party and
//! party-leader stores still carry per country after quiet hours.
//!
//! The domains are each test's own (`Watches`), so parallel tests never share
//! a token; the hooks' global domains are exercised in
//! [`crate::political_status_changes`].
use super::*;
use crate::{
    character_changes, party_changes, party_leader_changes,
    political_status_changes::{self, Counts},
    sampled_verify::Check,
};
use engine::{
    ccharacter, ccountry, ccountry_leader, cgame_state, cidea, cideology_group, cpolitical_party,
    memory::SliceMemory, msvc_string, political_status_mutations::Domain,
};
use std::cell::RefCell;

/// The game state at zero with its country table and links; each country in
/// its own region, the `CCountry` at its start (its fields end below 0x1600)
/// and its politics objects from [`POLITICS`] on.
const GS: usize = 0;
const CTAB: usize = 0x1000;
const LTAB: usize = 0x1200;
const FIRST: usize = 0x4000;
const REGION: usize = 0x4000;
const POLITICS: usize = 0x2000;
/// The sentinel, two countries and a slot a dynamic country takes later.
const SLOTS: usize = 4;
const COUNTRIES: usize = 3;
const SIZE: usize = FIRST + COUNTRIES * REGION;

fn country(index: usize) -> usize {
    FIRST + index * REGION
}
fn status(index: usize) -> usize {
    country(index) + POLITICS
}
fn party_array(index: usize) -> usize {
    status(index) + 0x100
}
fn idea_array(index: usize) -> usize {
    status(index) + 0x140
}
fn party(index: usize, which: usize) -> usize {
    status(index) + 0x200 + which * 0x100
}
fn ideology(index: usize, which: usize) -> usize {
    status(index) + 0x400 + which * 0x80
}
fn idea(index: usize, which: usize) -> usize {
    status(index) + 0x500 + which * 0x100
}
fn leader_vector(index: usize, which: usize) -> usize {
    status(index) + 0x700 + which * 0x20
}
fn leader(index: usize, which: usize) -> usize {
    status(index) + 0x800 + which * 0x160
}
fn character(index: usize, which: usize) -> usize {
    status(index) + 0xb00 + which * 0x100
}
/// A character's leader map head; its one node follows at +0x40.
fn map(index: usize, which: usize) -> usize {
    status(index) + 0xd00 + which * 0x80
}

struct World {
    bytes: Vec<u8>,
}

impl World {
    /// Countries 1 and 2 in slots 1 and 2, and country 3's objects with slot
    /// 3 still empty. Each country: parties 3 (ruling) and 7, each with one
    /// leader whose character lists it under role key -7, and ideas `idea_a`
    /// and `idea_b`.
    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; SIZE],
        };
        let header = ((SLOTS as u64) << 32) | SLOTS as u64;
        world.word(GS + cgame_state::COUNTRIES, CTAB as u64);
        world.word(GS + cgame_state::COUNTRIES + 8, header);
        world.word(GS + cgame_state::COUNTRY_LINKS, LTAB as u64);
        world.word(GS + cgame_state::COUNTRY_LINKS + 8, header);
        for slot in 0..SLOTS {
            world.int(LTAB + slot * 4, slot as i32);
        }
        for index in 0..COUNTRIES {
            world.country(index);
        }
        world.word(CTAB + 8, country(0) as u64);
        world.word(CTAB + 16, country(1) as u64);
        world
    }

    fn word(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn int(&mut self, at: usize, value: i32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// An inline MSVC string.
    fn string(&mut self, at: usize, text: &[u8]) {
        assert!(text.len() <= 15);
        self.bytes[at..at + 16].fill(0);
        self.bytes[at..at + text.len()].copy_from_slice(text);
        self.word(at + msvc_string::LENGTH, text.len() as u64);
        self.word(at + msvc_string::CAPACITY, msvc_string::INLINE_CAPACITY);
    }

    fn popularity(&mut self, index: usize, which: usize, value: i64) {
        self.word(
            party(index, which) + cpolitical_party::POPULARITY,
            value as u64,
        );
    }

    /// The role key under which a character's leader map lists its leader.
    fn role(&mut self, index: usize, which: usize, key: i32) {
        self.word(map(index, which) + 0x40 + 0x20, u64::from(key as u32));
    }

    fn country(&mut self, index: usize) {
        let at = country(index);
        let status = status(index);
        self.word(at, ccountry::VTABLE_RVA as u64);
        self.int(at + ccountry::ID, index as i32 + 1);
        self.word(at + ccountry::POLITICAL_STATUS, status as u64);
        self.int(at + ccountry::CAPITAL, 100 + index as i32);
        self.int(at + ccountry::ORIGINAL_CAPITAL, 200 + index as i32);
        self.word(status, cpolitical_status::VTABLE_RVA as u64);
        self.word(status + cpolitical_status::COUNTRY, at as u64);
        self.word(
            status + cpolitical_status::PARTIES,
            party_array(index) as u64,
        );
        self.word(status + cpolitical_status::PARTIES + 8, (2 << 32) | 2);
        self.word(status + cpolitical_status::IDEAS, idea_array(index) as u64);
        self.word(status + cpolitical_status::IDEAS + 8, (2 << 32) | 2);
        self.word(
            status + cpolitical_status::RULING_PARTY,
            party(index, 0) as u64,
        );
        self.int(status + cpolitical_status::LAST_ELECTION, 1_000);
        self.int(status + cpolitical_status::ELECTION_FREQUENCY, 48);
        self.bytes[status + cpolitical_status::ELECTIONS_ALLOWED] = 1;
        for which in 0..2 {
            let id = [3_u64, 7][which];
            let party = party(index, which);
            let ideology = ideology(index, which);
            self.word(party_array(index) + which * 8, party as u64);
            self.word(party, cpolitical_party::VTABLE_RVA as u64);
            self.word(party + cpolitical_party::ID, id);
            self.word(party + cpolitical_party::IDEOLOGY_GROUP, ideology as u64);
            self.word(ideology, cideology_group::VTABLE_RVA as u64);
            self.word(ideology + cideology_group::ID, id);
            self.string(
                ideology + cideology_group::TOKEN,
                [b"democratic".as_slice(), b"fascism"][which],
            );
            self.word(party + cpolitical_party::TAG, 40 + index as u64);
            self.string(party + cpolitical_party::NAME, b"PARTY");
            self.string(party + cpolitical_party::LONG_NAME, b"PARTY_LONG");
            self.bytes[party + cpolitical_party::NAME_DEFAULT] = 0;
            self.popularity(index, which, 50_000 * (which as i64 + 1));
            let (vector, leader, character) = (
                leader_vector(index, which),
                leader(index, which),
                character(index, which),
            );
            let (head, node) = (map(index, which), map(index, which) + 0x40);
            self.word(party + cpolitical_party::COUNTRY_LEADERS, vector as u64);
            self.word(party + cpolitical_party::COUNTRY_LEADERS + 8, (1 << 32) | 1);
            self.word(vector, leader as u64);
            self.word(leader, ccountry_leader::VTABLE_RVA as u64);
            self.word(leader + ccountry_leader::CHARACTER, character as u64);
            self.word(leader + ccountry_leader::LEADER_ID, 12 + which as u64);
            self.word(character, ccharacter::VTABLE_RVA as u64);
            self.word(
                character + ccharacter::TYPE,
                ((91 + 10 * index as u64 + which as u64) << 32)
                    | u64::from(ccharacter::REFERENCE_TYPE),
            );
            self.word(character + ccharacter::LEADER_MAP_HEAD, head as u64);
            self.word(character + ccharacter::LEADER_MAP_SIZE, 1);
            for offset in [0, 8, 16] {
                self.word(head + offset, node as u64);
                self.word(node + offset, head as u64);
            }
            self.word(head + 0x18, 1 << 8);
            self.role(index, which, -7);
            self.word(node + 0x28, leader as u64);
            let idea = idea(index, which);
            self.word(idea_array(index) + which * 8, idea as u64);
            self.word(idea, cidea::VTABLE_RVA as u64);
            self.string(
                idea + cidea::TOKEN,
                [b"idea_a".as_slice(), b"idea_b"][which],
            );
        }
    }
}

/// Every guarded copy's address and length.
struct Counted<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Counted<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        self.memory.copy(address, output)
    }
}

/// Reads of the fields only the Politics walk reads, beyond the status
/// authentication TimedIdeas shares: the capital, the status's party and
/// idea arrays, last election and ruling party, and every
/// party, ideology, idea, leader, character and leader map; and separately
/// reads of exactly one party's popularity.
fn classify(reads: &[(usize, usize)]) -> (usize, usize) {
    let (mut politics, mut popularity) = (0, 0);
    for &(address, len) in reads {
        let Some(offset) = address.checked_sub(FIRST) else {
            continue;
        };
        if offset / REGION >= COUNTRIES {
            continue;
        }
        let at = offset % REGION;
        let overlaps = |start: usize, end: usize| at < end && start < at + len;
        let s = POLITICS;
        if len == 8
            && (0..2).any(|which| at == s + 0x200 + which * 0x100 + cpolitical_party::POPULARITY)
        {
            popularity += 1;
        } else if overlaps(ccountry::CAPITAL, ccountry::ORIGINAL_CAPITAL + 4)
            || overlaps(
                s + cpolitical_status::PARTIES,
                s + cpolitical_status::PARTY_COUNT + 4,
            )
            || overlaps(
                s + cpolitical_status::IDEAS,
                s + cpolitical_status::IDEA_COUNT + 4,
            )
            || overlaps(
                s + cpolitical_status::LAST_ELECTION,
                s + cpolitical_status::LAST_ELECTION + 4,
            )
            || overlaps(
                s + cpolitical_status::RULING_PARTY,
                s + cpolitical_status::RULING_PARTY + 8,
            )
            || overlaps(s + 0x100, REGION)
        {
            politics += 1;
        }
    }
    (politics, popularity)
}

struct Domains {
    status: political_status_changes::Watches,
    parties: party_changes::Watches,
    leaders: party_leader_changes::Watches,
    characters: character_changes::Watches,
}

impl Domains {
    fn new() -> Self {
        Self {
            status: political_status_changes::Watches::enabled(),
            parties: party_changes::Watches::enabled(64),
            leaders: party_leader_changes::Watches::enabled(64),
            characters: character_changes::Watches::enabled(64),
        }
    }

    fn boundary(&self, invalidate: bool) {
        self.status.boundary(invalidate);
        self.parties.boundary(invalidate);
        self.leaders.boundary(invalidate);
        self.characters.boundary(invalidate);
    }
}

/// What one hour published.
struct Published {
    /// Politics, PoliticalParties, ActiveIdeas and PartyLeaders.
    tables: Vec<TableView>,
    buffers: Buffered,
    counts: Counts,
    politics_reads: usize,
    popularity_reads: usize,
}

/// The four tables' buffers as the spool reads them.
#[derive(Debug, PartialEq)]
struct Buffered {
    outcomes: [Outcome; 4],
    politics: Vec<(u32, cpolitical_status::Row)>,
    parties: Vec<cpolitical_party::Row>,
    party_text: (Vec<u8>, usize, usize),
    party_countries: (Vec<u32>, usize, usize),
    ideas: Vec<cpolitical_status::IdeaRow>,
    idea_text: (Vec<u8>, usize, usize),
    idea_countries: (Vec<u32>, usize, usize),
    leaders: Vec<cpolitical_party::LeaderRow>,
}

fn arena<T: Clone>(arena: &crate::table_capture::Arena<T>) -> (Vec<T>, usize, usize) {
    (arena.first[..arena.len].to_vec(), arena.len, arena.required)
}

fn buffered(captured: &Captured) -> Buffered {
    Buffered {
        outcomes: [
            captured.politics.outcome,
            captured.parties.outcome,
            captured.ideas.outcome,
            captured.party_leaders.outcome,
        ],
        politics: captured.politics.rows[..captured.politics.outcome.len()]
            .iter()
            .map(|row| (row.country, row.row))
            .collect(),
        parties: captured.parties.rows[..captured.parties.outcome.len()].to_vec(),
        party_text: arena(&captured.party_text),
        party_countries: arena(&captured.party_countries),
        ideas: captured.ideas.rows[..captured.ideas.outcome.len()].to_vec(),
        idea_text: arena(&captured.idea_text),
        idea_countries: arena(&captured.idea_countries),
        leaders: captured.party_leaders.rows[..captured.party_leaders.outcome.len()].to_vec(),
    }
}

fn tables(captured: &Captured) -> Vec<TableView> {
    let all = captured.tables();
    [5, 6, 7, 21]
        .iter()
        .map(|&index| all[index].clone())
        .collect()
}

/// The old behaviour: the same memory walked with no store.
fn walked(world: &World, hour: i32) -> (Vec<TableView>, Buffered) {
    let memory = SliceMemory {
        base: 0,
        bytes: &world.bytes,
    };
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, 0, GS, hour, false);
    (tables(&captured), buffered(&captured))
}

fn politics(published: &Published) -> &[rows::PoliticsRow] {
    let TableView::Complete(TableRows::Politics(rows)) = &published.tables[0] else {
        panic!("Politics unavailable: {:?}", published.tables[0]);
    };
    rows
}

fn parties(published: &Published) -> &[rows::PoliticalPartyRow] {
    let TableView::Complete(TableRows::PoliticalParties(rows)) = &published.tables[1] else {
        panic!("PoliticalParties unavailable: {:?}", published.tables[1]);
    };
    rows
}

/// One Governance job's stores across hours.
struct Session<'d> {
    domains: &'d Domains,
    captured: Captured,
    parties: PartyStore,
    leaders: LeaderStore,
    fingerprints: SliceFingerprints,
}

impl<'d> Session<'d> {
    fn new(domains: &'d Domains) -> Self {
        Self {
            domains,
            captured: Captured::with_capacities(&Capacities::default()),
            parties: PartyStore::new(),
            leaders: LeaderStore::new(),
            fingerprints: SliceFingerprints::new(),
        }
    }

    /// One hour, `check` choosing the quiet hour's countries read afresh
    /// (`Check::Every` stands for `political_status_verify`), then the next
    /// boundary.
    fn hour(&mut self, world: &World, hour: i32, check: Check) -> Published {
        let memory = Counted {
            memory: SliceMemory {
                base: 0,
                bytes: &world.bytes,
            },
            reads: RefCell::default(),
        };
        let pair = party_leader_changes::Pair {
            parties: &self.domains.leaders,
            characters: &self.domains.characters,
        };
        let politics = PoliticsWalk {
            leaders: Some(&mut self.leaders),
            parties: Some(&mut self.parties),
            leader_changes: &pair,
            party_changes: &self.domains.parties,
            status: &self.domains.status,
            leader_check: Check::Nothing,
            party_check: Check::Nothing,
            check,
            force_full: false,
        };
        self.captured.capture_sliced(
            &memory,
            0,
            GS,
            hour,
            false,
            Slices {
                technologies: &mut self.fingerprints,
                focus: &mut SliceFingerprints::new(),
                changes: &crate::country_input_changes::Observer,
            },
            Some(politics),
        );
        let (politics_reads, popularity_reads) = classify(&memory.reads.borrow());
        let published = Published {
            tables: tables(&self.captured),
            buffers: buffered(&self.captured),
            counts: self.parties.quiet_counts(),
            politics_reads,
            popularity_reads,
        };
        self.domains.boundary(false);
        published
    }

    /// The first hour walks every country and holds its tables; the second is
    /// quiet.
    fn start(&mut self, world: &World) {
        let first = self.hour(world, 1, Check::Nothing);
        assert!(first.counts.read && !first.counts.quiet);
        assert_ne!(first.politics_reads, 0, "a walk reads every country");
        assert_eq!((first.tables, first.buffers), walked(world, 1));
        let quiet = self.hour(world, 2, Check::Nothing);
        assert!(quiet.counts.quiet && !quiet.counts.read);
    }
}

#[test]
fn a_quiet_hour_publishes_what_a_walk_of_the_same_memory_publishes() {
    let world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    for hour in 3..6 {
        let quiet = session.hour(&world, hour, Check::Nothing);
        assert!(quiet.counts.quiet, "hour {hour}");
        assert_eq!(politics(&quiet).len(), 2);
        assert_eq!(parties(&quiet).len(), 4);
        // The same tables and the same buffers, arenas and demand.
        assert_eq!((quiet.tables, quiet.buffers), walked(&world, hour));
    }
}

#[test]
fn a_quiet_hour_reads_no_country_politics() {
    let world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    for hour in 3..8 {
        let quiet = session.hour(&world, hour, Check::Nothing);
        assert!(quiet.counts.quiet, "hour {hour}");
        assert_eq!(
            (quiet.politics_reads, quiet.popularity_reads),
            (0, 0),
            "hour {hour}"
        );
    }
}

/// A named structure write and how a test world performs it.
type Write = (&'static str, fn(&mut World));
#[test]
fn each_hooked_structure_write_walks_and_publishes_its_value() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    let writes: [Write; 5] = [
        ("capital", |world| {
            world.int(country(0) + ccountry::CAPITAL, 555)
        }),
        ("ruling party", |world| {
            world.word(
                status(1) + cpolitical_status::RULING_PARTY,
                party(1, 1) as u64,
            )
        }),
        // Each array header's word at +8 is the capacity (low) and the count
        // (high): keep capacity 2 and list one element. A count above the
        // capacity is an invalid header, which refuses the table, so that
        // hour's walk is incomplete and holds nothing.
        ("party array", |world| {
            world.word(status(0) + cpolitical_status::PARTIES + 8, (1 << 32) | 2)
        }),
        ("idea array", |world| {
            world.word(status(1) + cpolitical_status::IDEAS + 8, (1 << 32) | 2)
        }),
        // Set politics stores the elections byte.
        ("elections", |world| {
            world.bytes[status(0) + cpolitical_status::ELECTIONS_ALLOWED] = 0
        }),
    ];
    let mut hour = 3;
    for (name, write) in writes {
        write(&mut world);
        domains.status.write(Domain::Structure);
        let walk = session.hour(&world, hour, Check::Nothing);
        assert!(walk.counts.read && !walk.counts.quiet, "{name}");
        assert_ne!(walk.politics_reads, 0, "{name}");
        assert_eq!((walk.tables, walk.buffers), walked(&world, hour), "{name}");
        hour += 1;
        // Held again: the next hour is quiet with the new values.
        let quiet = session.hour(&world, hour, Check::Nothing);
        assert!(quiet.counts.quiet, "{name}");
        assert_eq!(quiet.politics_reads, 0, "{name}");
        assert_eq!(
            (quiet.tables, quiet.buffers),
            walked(&world, hour),
            "{name}"
        );
        hour += 1;
    }
    let last = session.hour(&world, hour, Check::Nothing);
    assert_eq!(politics(&last)[0].capital, 555);
    assert_eq!(politics(&last)[1].ruling_party, Some(7));
    assert_eq!(
        politics(&last)
            .iter()
            .map(|row| row.has_elections)
            .collect::<Vec<_>>(),
        [rows::Recorded::Value(false), rows::Recorded::Value(true)]
    );
    assert_eq!(parties(&last).len(), 3, "country 1 lists one party");
    let TableView::Complete(TableRows::ActiveIdeas(ideas)) = &last.tables[2] else {
        panic!("ActiveIdeas unavailable");
    };
    assert_eq!(ideas.len(), 3, "country 2 lists one idea");
}

#[test]
fn a_popularity_write_gathers_only_popularity() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    // AddPopularity (or the daily update) on country 2's second party.
    world.popularity(1, 1, 77_777);
    domains.status.write(Domain::Popularity);
    let refreshed = session.hour(&world, 3, Check::Nothing);
    assert!(refreshed.counts.popularity, "a popularity refresh hour");
    assert!(!refreshed.counts.quiet && !refreshed.counts.read);
    assert_eq!(refreshed.politics_reads, 0, "only +0x88 is read");
    assert_eq!(refreshed.popularity_reads, 4, "once per held party");
    assert_eq!(parties(&refreshed)[3].popularity, 77_777);
    assert_eq!((refreshed.tables, refreshed.buffers), walked(&world, 3));
    // The new popularity is held under the new token.
    let quiet = session.hour(&world, 4, Check::Nothing);
    assert!(quiet.counts.quiet);
    assert_eq!((quiet.politics_reads, quiet.popularity_reads), (0, 0));
    assert_eq!(parties(&quiet)[3].popularity, 77_777);
    // A structure write still walks.
    domains.status.write(Domain::Both);
    let walk = session.hour(&world, 5, Check::Nothing);
    assert!(walk.counts.read);
}

#[test]
fn a_change_no_hook_saw_is_caught_by_the_slice_poisons_and_walks() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    // Country 2 falls in hour 2's slice, country 1 in hour 1's.
    let (first, second) = (Check::Slice { hour: 1 }, Check::Slice { hour: 2 });
    assert!(first.covers(1) && !first.covers(2) && second.covers(2));
    let agreed = session.hour(&world, 3, second);
    assert!(agreed.counts.quiet);
    assert_eq!((agreed.counts.verified, agreed.counts.mismatches), (1, 0));
    assert_ne!(agreed.politics_reads, 0, "the slice reads its country");
    assert!(!domains.status.poisoned());
    // A capital write on country 1 that no hook saw.
    world.int(country(0) + ccountry::CAPITAL, 999);
    let caught = session.hour(&world, 4, first);
    assert_eq!((caught.counts.verified, caught.counts.mismatches), (1, 1));
    assert!(caught.counts.read && !caught.counts.quiet);
    assert!(domains.status.poisoned());
    assert_eq!(politics(&caught)[0].capital, 999);
    assert_eq!((caught.tables, caught.buffers), walked(&world, 4));
    // Poisoned: every later hour walks.
    let after = session.hour(&world, 5, Check::Nothing);
    assert!(after.counts.read && !after.counts.quiet);
}

#[test]
fn verify_walks_every_hour_and_catches_a_change_no_hook_saw() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    let first = session.hour(&world, 1, Check::Every);
    assert!(first.counts.read);
    assert_eq!(first.counts.verified, 0, "nothing held yet");
    let agreed = session.hour(&world, 2, Check::Every);
    assert!(agreed.counts.read && !agreed.counts.quiet);
    assert_ne!(agreed.politics_reads, 0);
    assert_eq!((agreed.counts.verified, agreed.counts.mismatches), (2, 0));
    assert!(!domains.status.poisoned());
    // A popularity write no hook saw: the token stands still.
    world.popularity(0, 0, 1);
    let caught = session.hour(&world, 3, Check::Every);
    assert_eq!((caught.counts.verified, caught.counts.mismatches), (2, 1));
    assert!(domains.status.poisoned());
    assert_eq!((caught.tables, caught.buffers), walked(&world, 3));
}

#[test]
fn a_revoked_leader_certificate_walks() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    // A role-map write on country 2's first leader's character, which only
    // the character domain observes.
    let notification = domains.characters.notify(Some(character(1, 0)), 2);
    world.role(1, 0, -8);
    drop(notification);
    let walk = session.hour(&world, 3, Check::Nothing);
    assert!(walk.counts.read && !walk.counts.quiet);
    let expected = walked(&world, 3);
    assert_eq!((&walk.tables, &walk.buffers), (&expected.0, &expected.1));
    let TableView::Complete(TableRows::PartyLeaders(leaders)) = &walk.tables[3] else {
        panic!("PartyLeaders unavailable");
    };
    assert_eq!(leaders[2].role, Some(-8));
    assert!(session.hour(&world, 4, Check::Nothing).counts.quiet);
}

#[test]
fn a_new_country_walks() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    // A dynamic country takes slot 3; no politics hook ran.
    world.word(CTAB + 3 * 8, country(2) as u64);
    let walk = session.hour(&world, 3, Check::Nothing);
    assert!(walk.counts.read && !walk.counts.quiet);
    assert_eq!(politics(&walk).len(), 3);
    assert_eq!((walk.tables, walk.buffers), walked(&world, 3));
    let quiet = session.hour(&world, 4, Check::Nothing);
    assert!(quiet.counts.quiet);
    assert_eq!(politics(&quiet).len(), 3);
}

#[test]
fn quiet_hours_leave_the_stores_carrying_per_country() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    assert!(session.hour(&world, 3, Check::Nothing).counts.quiet);
    // set_party_name on country 2's second party: only the party domain sees
    // it, so the hour walks and the stores carry every other country.
    let notification = domains.parties.notify(party(1, 1), 0);
    world.string(party(1, 1) + cpolitical_party::NAME, b"RENAMED");
    drop(notification);
    let walk = session.hour(&world, 4, Check::Nothing);
    assert!(walk.counts.read && !walk.counts.quiet);
    assert_eq!((walk.tables, walk.buffers), walked(&world, 4));
    let [parties, ideas] = session.parties.counts();
    assert_eq!((parties.read, parties.reused), (1, 1));
    assert_eq!((ideas.read, ideas.reused), (0, 2));
    let leaders = session.leaders.counts();
    assert_eq!((leaders.read, leaders.reused), (0, 2));
    assert!(session.hour(&world, 5, Check::Nothing).counts.quiet);
}

#[test]
fn an_invalidating_hour_or_a_forgotten_store_walks() {
    let world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    // A load, gap or recovery starts an epoch in every domain.
    domains.boundary(true);
    let reloaded = session.hour(&world, 3, Check::Nothing);
    assert!(reloaded.counts.read);
    assert!(session.hour(&world, 4, Check::Nothing).counts.quiet);
    // A failed Governance job forgets both stores.
    session.parties.forget();
    session.leaders.forget();
    let forgotten = session.hour(&world, 5, Check::Nothing);
    assert!(forgotten.counts.read);
    assert_eq!((forgotten.tables, forgotten.buffers), walked(&world, 5));
    assert!(session.hour(&world, 6, Check::Nothing).counts.quiet);
}
