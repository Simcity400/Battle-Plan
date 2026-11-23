use super::*;
use crate::{
    character_changes, party_changes, party_leader_changes,
    political_status_changes::{self, Counts},
};
use engine_mutations::political_status_mutations::Domain;
use engine_native::{ccharacter, ccountry, cgame_state};
use engine_native_parts::{cidea, cideology_group};
use engine_native_memory::{memory::SliceMemory, msvc_string};
use engine_native_politics::{ccountry_leader, cpolitical_party};
use std::cell::RefCell;

const GS: usize = 0;
const CTAB: usize = 0x1000;
const LTAB: usize = 0x1200;
const FIRST: usize = 0x4000;
const REGION: usize = 0x4000;
const POLITICS: usize = 0x2000;
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
fn map(index: usize, which: usize) -> usize {
    status(index) + 0xd00 + which * 0x80
}

struct World {
    bytes: Vec<u8>,
}

impl World {
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

    fn role(&mut self, index: usize, which: usize, key: i32) {
        self.word(map(index, which) + 0x40 + 0x20, u64::from(key as u32));
    }

    fn country(&mut self, index: usize) {
        let at = country(index);
        let status = status(index);
        self.word(at, ccountry::VTABLE_RVA.0 as u64);
        self.int(at + ccountry::ID, index as i32 + 1);
        self.word(at + ccountry::POLITICAL_STATUS, status as u64);
        self.int(at + ccountry::CAPITAL, 100 + index as i32);
        self.int(at + ccountry::ORIGINAL_CAPITAL, 200 + index as i32);
        self.word(status, engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0 as u64);
        self.word(status + engine_native_parts::cpolitical_status_layout::COUNTRY, at as u64);
        self.word(
            status + cpolitical_status::PARTIES,
            party_array(index) as u64,
        );
        self.word(status + cpolitical_status::PARTIES + 8, (2 << 32) | 2);
        self.word(status + cpolitical_status::IDEAS, idea_array(index) as u64);
        self.word(status + cpolitical_status::IDEAS + 8, (2 << 32) | 2);
        self.word(
            status + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
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
            self.word(party, engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0 as u64);
            self.word(party + engine_native_parts::cpolitical_party_layout::ID, id);
            self.word(party + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, ideology as u64);
            self.word(ideology, cideology_group::VTABLE_RVA.0 as u64);
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
            self.word(party + engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS, vector as u64);
            self.word(party + engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS + 8, (1 << 32) | 1);
            self.word(vector, leader as u64);
            self.word(leader, engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0 as u64);
            self.word(leader + engine_native_parts::ccountry_leader_layout::CHARACTER, character as u64);
            self.word(leader + ccountry_leader::LEADER_ID, 12 + which as u64);
            self.word(character, ccharacter::VTABLE_RVA.0 as u64);
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
            self.word(idea, cidea::VTABLE_RVA.0 as u64);
            self.string(
                idea + cidea::TOKEN,
                [b"idea_a".as_slice(), b"idea_b"][which],
            );
        }
    }
}

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
                s + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
                s + engine_native_parts::cpolitical_status_layout::RULING_PARTY + 8,
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

struct Published {
    tables: Vec<TableView>,
    buffers: Buffered,
    counts: Counts,
    politics_reads: usize,
    popularity_reads: usize,
}

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

    fn hour(&mut self, world: &World, hour: i32) -> Published {
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

    fn start(&mut self, world: &World) {
        let first = self.hour(world, 1);
        assert!(first.counts.read && !first.counts.quiet);
        assert_ne!(first.politics_reads, 0, "a walk reads every country");
        assert_eq!((first.tables, first.buffers), walked(world, 1));
        let quiet = self.hour(world, 2);
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
        let quiet = session.hour(&world, hour);
        assert!(quiet.counts.quiet, "hour {hour}");
        assert_eq!(politics(&quiet).len(), 2);
        assert_eq!(parties(&quiet).len(), 4);
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
        let quiet = session.hour(&world, hour);
        assert!(quiet.counts.quiet, "hour {hour}");
        assert_eq!(
            (quiet.politics_reads, quiet.popularity_reads),
            (0, 0),
            "hour {hour}"
        );
    }
}

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
                status(1) + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
                party(1, 1) as u64,
            )
        }),
        ("party array", |world| {
            world.word(status(0) + cpolitical_status::PARTIES + 8, (1 << 32) | 2)
        }),
        ("idea array", |world| {
            world.word(status(1) + cpolitical_status::IDEAS + 8, (1 << 32) | 2)
        }),
        ("elections", |world| {
            world.bytes[status(0) + cpolitical_status::ELECTIONS_ALLOWED] = 0
        }),
    ];
    let mut hour = 3;
    for (name, write) in writes {
        write(&mut world);
        domains.status.write(Domain::Structure);
        let walk = session.hour(&world, hour);
        assert!(walk.counts.read && !walk.counts.quiet, "{name}");
        assert_ne!(walk.politics_reads, 0, "{name}");
        assert_eq!((walk.tables, walk.buffers), walked(&world, hour), "{name}");
        hour += 1;
        let quiet = session.hour(&world, hour);
        assert!(quiet.counts.quiet, "{name}");
        assert_eq!(quiet.politics_reads, 0, "{name}");
        assert_eq!(
            (quiet.tables, quiet.buffers),
            walked(&world, hour),
            "{name}"
        );
        hour += 1;
    }
    let last = session.hour(&world, hour);
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
    world.popularity(1, 1, 77_777);
    domains.status.write(Domain::Popularity);
    let refreshed = session.hour(&world, 3);
    assert!(refreshed.counts.popularity, "a popularity refresh hour");
    assert!(!refreshed.counts.quiet && !refreshed.counts.read);
    assert_eq!(refreshed.politics_reads, 0, "only the popularity is read");
    assert_eq!(refreshed.popularity_reads, 4, "once per held party");
    assert_eq!(parties(&refreshed)[3].popularity, 77_777);
    assert_eq!((refreshed.tables, refreshed.buffers), walked(&world, 3));
    let quiet = session.hour(&world, 4);
    assert!(quiet.counts.quiet);
    assert_eq!((quiet.politics_reads, quiet.popularity_reads), (0, 0));
    assert_eq!(parties(&quiet)[3].popularity, 77_777);
    domains.status.write(Domain::Both);
    let walk = session.hour(&world, 5);
    assert!(walk.counts.read);
}

#[test]
fn a_revoked_leader_certificate_walks() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    let notification = domains.characters.notify(Some(character(1, 0)), 2);
    world.role(1, 0, -8);
    drop(notification);
    let walk = session.hour(&world, 3);
    assert!(walk.counts.read && !walk.counts.quiet);
    let expected = walked(&world, 3);
    assert_eq!((&walk.tables, &walk.buffers), (&expected.0, &expected.1));
    let TableView::Complete(TableRows::PartyLeaders(leaders)) = &walk.tables[3] else {
        panic!("PartyLeaders unavailable");
    };
    assert_eq!(leaders[2].role, Some(-8));
    assert!(session.hour(&world, 4).counts.quiet);
}

#[test]
fn a_new_country_walks() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    world.word(CTAB + 3 * 8, country(2) as u64);
    let walk = session.hour(&world, 3);
    assert!(walk.counts.read && !walk.counts.quiet);
    assert_eq!(politics(&walk).len(), 3);
    assert_eq!((walk.tables, walk.buffers), walked(&world, 3));
    let quiet = session.hour(&world, 4);
    assert!(quiet.counts.quiet);
    assert_eq!(politics(&quiet).len(), 3);
}

#[test]
fn quiet_hours_leave_the_stores_carrying_per_country() {
    let mut world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    assert!(session.hour(&world, 3).counts.quiet);
    let notification = domains.parties.notify(party(1, 1), 0);
    world.string(party(1, 1) + cpolitical_party::NAME, b"RENAMED");
    drop(notification);
    let walk = session.hour(&world, 4);
    assert!(walk.counts.read && !walk.counts.quiet);
    assert_eq!((walk.tables, walk.buffers), walked(&world, 4));
    let [parties, ideas] = session.parties.counts();
    assert_eq!((parties.read, parties.reused), (1, 1));
    assert_eq!((ideas.read, ideas.reused), (0, 2));
    let leaders = session.leaders.counts();
    assert_eq!((leaders.read, leaders.reused), (0, 2));
    assert!(session.hour(&world, 5).counts.quiet);
}

#[test]
fn an_invalidating_hour_or_a_forgotten_store_walks() {
    let world = World::new();
    let domains = Domains::new();
    let mut session = Session::new(&domains);
    session.start(&world);
    domains.boundary(true);
    let reloaded = session.hour(&world, 3);
    assert!(reloaded.counts.read);
    assert!(session.hour(&world, 4).counts.quiet);
    session.parties.forget();
    session.leaders.forget();
    let forgotten = session.hour(&world, 5);
    assert!(forgotten.counts.read);
    assert_eq!((forgotten.tables, forgotten.buffers), walked(&world, 5));
    assert!(session.hour(&world, 6).counts.quiet);
}
