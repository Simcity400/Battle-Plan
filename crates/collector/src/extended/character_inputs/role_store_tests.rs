//! CharacterRoleTraits over the roster job's role census with held role rows
//! ([`super::RoleStore`], [`crate::character_changes`]): every country of
//! the populated fixture holds an active character with a country-leader,
//! advisor and scientist role and a retired character with empty maps.

use super::scale_tests::{BASE, MANAGER, Populated, STATE, TRAITS, resolved_operatives};
use super::{Capacities, Captured, RoleStore};
use crate::character_changes::{Changes, Counting as Lookups, Counts, Watches};
use crate::extended::characters::census::RoleCensus;
use engine_characters::cscientist;
use engine_native::{
    Memory, ccharacter, ccharacter_manager, ccountry_leader, cgame_state, cpdx_array::Array,
    cunit_leader, walk::Outcome,
};
use replay_data::{rows::CharacterRoleTraitRow, rows::TableRows};
use replay_frame::frame::TableView;
use std::{cell::Cell, ops::Range};

const COUNTRIES: usize = 40;
const CHARACTERS: u64 = COUNTRIES as u64 * 2;

/// Counts reads that touch any role map, role object or role trait array.
struct Watched<'a> {
    memory: &'a Populated,
    roles: Vec<Range<usize>>,
    hits: Cell<usize>,
}

impl Memory for Watched<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let end = address.saturating_add(output.len());
        if self
            .roles
            .iter()
            .any(|range| address < range.end && end > range.start)
        {
            self.hits.set(self.hits.get() + 1);
        }
        self.memory.copy(address, output)
    }
}

struct World {
    memory: Populated,
    /// Roster occurrences in order: each country's active then retired
    /// character.
    characters: Vec<usize>,
}

/// A leader domain never observed: no operative link is carried.
static UNOBSERVED: crate::unit_leader_changes::Watches = crate::unit_leader_changes::Watches::new();

impl World {
    fn new() -> Self {
        Self::with(COUNTRIES)
    }

    fn with(countries: usize) -> Self {
        Self::of(Populated::new(countries))
    }

    fn of(memory: Populated) -> Self {
        let mut header = [0; 16];
        assert!(memory.copy(
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            &mut header
        ));
        let roster = Array::from_header(header, 8).unwrap();
        let characters = (0..roster.count as usize)
            .map(|index| {
                let mut pointer = [0; 8];
                assert!(memory.copy(roster.data + index * 8, &mut pointer));
                u64::from_le_bytes(pointer) as usize
            })
            .collect();
        Self { memory, characters }
    }

    /// The roster job's census of this callback.
    fn census(&self, memory: &impl Memory) -> RoleCensus {
        let identities: Vec<_> = self
            .characters
            .iter()
            .map(|&pointer| {
                let mut id = [0; 4];
                assert!(memory.copy(pointer + ccharacter::ID, &mut id));
                ccharacter_manager::PortraitIdentity {
                    pointer,
                    character: u32::from_le_bytes(id),
                }
            })
            .collect();
        let mut census = RoleCensus::new(identities.len());
        assert!(census.gather(memory, (BASE, STATE), &identities, identities.len()));
        census
    }

    /// Every active character's role memory (maps, roles, their trait
    /// arrays), excluding the scientist read every hour.
    fn roles(&self) -> Vec<Range<usize>> {
        self.characters
            .iter()
            .step_by(2)
            .map(|&active| {
                let block = active - 0x1100;
                block + 0x1300..block + 0x1580
            })
            .collect()
    }

    /// The country-leader role of the `country`th active character.
    fn leader(&self, country: usize) -> usize {
        self.characters[country * 2] - 0x1100 + 0x1440
    }

    /// Every active character's scientist object (its trait array header).
    fn scientists(&self) -> Vec<Range<usize>> {
        self.characters
            .iter()
            .step_by(2)
            .map(|&active| {
                let mut pointer = [0; 8];
                assert!(
                    self.memory
                        .copy(active + ccharacter::SCIENTIST, &mut pointer)
                );
                let scientist = u64::from_le_bytes(pointer) as usize;
                assert_ne!(scientist, 0);
                scientist..scientist + cscientist::TRAIT_COUNT + 4
            })
            .collect()
    }

    /// One hour's capture: its rows, its counts and its role-memory reads.
    fn hour(
        &self,
        store: &mut RoleStore,
        changes: &dyn Changes,
    ) -> (Vec<CharacterRoleTraitRow>, Counts, usize) {
        self.hour_with(store, changes, self.roles())
    }

    /// [`Self::hour`], counting the reads of `watched` instead.
    fn hour_with(
        &self,
        store: &mut RoleStore,
        changes: &dyn Changes,
        watched: Vec<Range<usize>>,
    ) -> (Vec<CharacterRoleTraitRow>, Counts, usize) {
        let census = self.census(&self.memory);
        let watched = Watched {
            memory: &self.memory,
            roles: watched,
            hits: Cell::new(0),
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture_census_with(
            &watched,
            BASE,
            STATE,
            1,
            false,
            census.admitted(BASE, STATE),
            Some(store),
            changes,
            &UNOBSERVED,
        );
        let [TableView::Complete(TableRows::CharacterRoleTraits(rows)), _] = captured.tables()
        else {
            panic!("complete character role traits");
        };
        (rows, store.counts(), watched.hits.get())
    }
}

fn tokens(rows: &[CharacterRoleTraitRow], character: u32) -> Vec<(u8, Vec<u32>)> {
    rows.iter()
        .filter(|row| row.character == character)
        .map(|row| (row.kind, row.tokens.clone()))
        .collect()
}

#[test]
fn unchanged_hours_read_no_character_and_publish_the_same_rows() {
    let world = World::new();
    let watches = Watches::enabled(world.characters.len());
    let mut store = RoleStore::new();
    let (first, counts, reads) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, counts.reused), (CHARACTERS, 0));
    assert!(reads > 0);
    // Leader, advisor and scientist rows of every active character.
    assert_eq!(first.len(), COUNTRIES * 3);
    for _ in 0..3 {
        watches.boundary(false);
        let (rows, counts, reads) = world.hour(&mut store, &watches);
        assert_eq!((counts.read, counts.reused), (0, CHARACTERS));
        assert_eq!(reads, 0, "an unchanged hour reads no role memory");
        assert_eq!(rows, first);
    }
}

#[test]
fn a_dirty_character_alone_is_read_afresh_and_exactly() {
    let mut world = World::new();
    let watches = Watches::enabled(world.characters.len());
    let mut store = RoleStore::new();
    let (first, ..) = world.hour(&mut store, &watches);
    // add_country_leader_trait on the third active character: its array now
    // holds the advisor-only trait, and its hooked writer names it.
    let leader = world.leader(2);
    let array = u64::from_le_bytes(
        world.memory.data[leader + ccountry_leader::TRAITS..][..8]
            .try_into()
            .unwrap(),
    ) as usize;
    world.memory.u64(array, (TRAITS + 0x10) as u64);
    let character = world.characters[4];
    drop(watches.notify(Some(character), 0));
    watches.boundary(false);
    let (rows, counts, _) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, counts.reused), (1, CHARACTERS - 1));
    let id = u32::from_le_bytes(
        world.memory.data[character + ccharacter::ID..][..4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        tokens(&rows, id),
        [
            (CharacterRoleTraitRow::COUNTRY_LEADER, vec![102]),
            (CharacterRoleTraitRow::ADVISOR, vec![102]),
            (CharacterRoleTraitRow::SCIENTIST, vec![201]),
        ]
    );
    let others = |rows: &[CharacterRoleTraitRow]| {
        rows.iter()
            .filter(|row| row.character != id)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(others(&rows), others(&first));
    // The next quiet hour carries the new rows.
    let (again, counts, reads) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, reads), (0, 0));
    assert_eq!(again, rows);
}

#[test]
fn world_invalidation_or_a_new_world_reads_every_character() {
    let world = World::new();
    let watches = Watches::enabled(world.characters.len());
    let mut store = RoleStore::new();
    let (first, ..) = world.hour(&mut store, &watches);
    watches.boundary(true);
    let (rows, counts, _) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, counts.reused), (CHARACTERS, 0));
    assert_eq!(rows, first);
    // A writer that cannot name its character revokes every certificate.
    drop(watches.notify(None, 0));
    let (_, counts, _) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, counts.reused), (CHARACTERS, 0));
    // A forgotten store (failed job, missing census) holds nothing.
    store.forget();
    let (_, counts, _) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, counts.reused), (CHARACTERS, 0));
    let (_, counts, _) = world.hour(&mut store, &watches);
    assert_eq!((counts.read, counts.reused), (0, CHARACTERS));
}

#[test]
fn quiet_hours_look_up_no_character_and_read_only_scientists() {
    let mut world = World::new();
    let domain = Lookups::new();
    domain.watches.enable(world.characters.len());
    let mut store = RoleStore::new();
    let (first, ..) = world.hour(&mut store, &domain);
    assert_eq!(domain.lookups(), CHARACTERS);
    let (rows, counts, reads) = world.hour(&mut store, &domain);
    assert_eq!(domain.lookups(), 0, "a quiet hour looks up no character");
    assert!(counts.quiet);
    assert_eq!((counts.read, counts.reused, reads), (0, CHARACTERS, 0));
    assert_eq!(rows, first);
    // A scientist trait writer moves the scientist token: the quiet hour
    // reads the scientist rows afresh and shows the change.
    let token = TRAITS + 0x20 + 8;
    domain.watches.scientist_write();
    world.memory.data[token..token + 4].copy_from_slice(&203_u32.to_le_bytes());
    let (rows, counts, _) = world.hour(&mut store, &domain);
    assert_eq!(domain.lookups(), 0);
    assert!(counts.quiet);
    let scientists = |rows: &[CharacterRoleTraitRow]| {
        rows.iter()
            .filter(|row| row.kind == CharacterRoleTraitRow::SCIENTIST)
            .map(|row| row.tokens.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(scientists(&rows), vec![vec![203]; COUNTRIES]);
    let others = |rows: &[CharacterRoleTraitRow]| {
        rows.iter()
            .filter(|row| row.kind != CharacterRoleTraitRow::SCIENTIST)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(others(&rows), others(&first));
    let current = rows;
    // A hooked writer: every character is looked up, and the next hour is
    // quiet again.
    drop(domain.watches.notify(Some(world.characters[4]), 0));
    let (rows, counts, _) = world.hour(&mut store, &domain);
    assert_eq!(domain.lookups(), CHARACTERS);
    assert!(!counts.quiet);
    assert_eq!(rows, current);
    world.hour(&mut store, &domain);
    assert_eq!(domain.lookups(), 0);
    // A census that changed (a character's country) is certified afresh.
    let character = world.characters[6] + ccharacter::COUNTRY;
    world.memory.data[character..character + 4].copy_from_slice(&77_u32.to_le_bytes());
    let (rows, counts, _) = world.hour(&mut store, &domain);
    assert_eq!(domain.lookups(), CHARACTERS);
    assert_eq!((counts.read, counts.reused), (0, CHARACTERS));
    assert_eq!(rows, current);
    // A load, gap or recovery reads every character.
    domain.watches.boundary(true);
    let (_, counts, _) = world.hour(&mut store, &domain);
    assert_eq!((counts.read, counts.reused), (CHARACTERS, 0));
}

#[test]
fn the_role_traits_walk_reads_no_roster() {
    // The Gated job walks the roster job's census: with the character
    // manager gone after the census was taken, the rows are still the ones
    // the roster walk reads.
    let mut world = World::new();
    let mut roster = Captured::with_capacities(&Capacities::default());
    roster.capture(&world.memory, BASE, STATE, 1, false);
    let expected = roster.tables();
    let census = world.census(&world.memory);
    world.memory.u64(STATE + cgame_state::CHARACTER_MANAGER, 0);
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture_census(
        &world.memory,
        BASE,
        STATE,
        1,
        false,
        census.admitted(BASE, STATE),
        None,
    );
    assert!(matches!(
        captured.outcomes()[0],
        Outcome::Rows { len } if len == COUNTRIES * 3
    ));
    assert_eq!(captured.tables(), expected);
    // Without a census the pack walks the roster, which is gone, and the
    // store forgets what it held.
    let mut store = RoleStore::new();
    captured.capture_census(&world.memory, BASE, STATE, 1, false, None, Some(&mut store));
    assert!(matches!(
        captured.outcomes()[0],
        Outcome::Unavailable { .. }
    ));
}

fn scientist_tokens(rows: &[CharacterRoleTraitRow]) -> Vec<Vec<u32>> {
    rows.iter()
        .filter(|row| row.kind == CharacterRoleTraitRow::SCIENTIST)
        .map(|row| row.tokens.clone())
        .collect()
}

/// A quiet hour carries the scientist rows too while no scientist role or
/// trait writer ran: it reads no scientist object, which the old quiet hour
/// read for every scientist each hour. A scientist writer re-reads them, and
/// the next quiet hour carries what it read.
#[test]
fn quiet_hours_carry_scientist_rows_until_a_scientist_writer_runs() {
    let mut world = World::new();
    let domain = Lookups::new();
    domain.watches.enable(world.characters.len());
    let mut store = RoleStore::new();
    let scientists = world.scientists();
    let (first, _, reads) = world.hour_with(&mut store, &domain, scientists.clone());
    assert_ne!(reads, 0);
    let (rows, counts, reads) = world.hour_with(&mut store, &domain, scientists.clone());
    assert!(counts.quiet);
    assert_eq!(reads, 0, "a quiet scientist hour reads no scientist");
    assert_eq!(rows, first);
    // The family's premise: a change no hook saw is carried stale...
    let token = TRAITS + 0x20 + 8;
    world.memory.data[token..token + 4].copy_from_slice(&203_u32.to_le_bytes());
    let (stale, _, reads) = world.hour_with(&mut store, &domain, scientists.clone());
    assert_eq!((stale, reads), (first.clone(), 0));
    // ...so every scientist writer is hooked: one re-reads every scientist.
    domain.watches.scientist_write();
    let (written, counts, reads) = world.hour_with(&mut store, &domain, scientists.clone());
    assert!(counts.quiet);
    assert_ne!(reads, 0);
    assert_eq!(scientist_tokens(&written), vec![vec![203]; COUNTRIES]);
    let (again, _, reads) = world.hour_with(&mut store, &domain, scientists.clone());
    assert_eq!((again, reads), (written, 0));
}

/// Ignored bench: CharacterRoleTraits' quiet hours at 13,200 characters
/// (6,600 countries): into a frame kept between hours, and into a fresh one.
///
/// `cargo test -p collector --release --lib role_traits_hour_costs -- --ignored --nocapture`
#[test]
#[ignore = "bench: run in release with --ignored --nocapture"]
fn role_traits_hour_costs_at_late_war_scale() {
    use std::time::{Duration, Instant};
    const HOURS: usize = 15;
    let world = World::with(6_600);
    let domain = Lookups::new();
    domain.watches.enable(world.characters.len());
    let census = world.census(&world.memory);
    let mut store = RoleStore::new();
    let timed = |captured: &mut Captured, store: &mut RoleStore| {
        captured.reset();
        let started = Instant::now();
        captured.capture_census_with(
            &world.memory,
            BASE,
            STATE,
            1,
            false,
            census.admitted(BASE, STATE),
            Some(store),
            &domain,
            &UNOBSERVED,
        );
        started.elapsed()
    };
    let hour = |store: &mut RoleStore| {
        timed(&mut Captured::with_capacities(&Capacities::default()), store)
    };
    for _ in 0..5 {
        hour(&mut store);
    }
    // A frame kept between hours holds the published table.
    let mut frame = Captured::with_capacities(&Capacities::default());
    for _ in 0..3 {
        timed(&mut frame, &mut store);
    }
    let mut kept: Vec<Duration> = (0..HOURS).map(|_| timed(&mut frame, &mut store)).collect();
    kept.sort_unstable();
    eprintln!("role traits quiet, kept frame: {:?}", kept[HOURS / 2]);
    let mut runs: Vec<Duration> = (0..HOURS).map(|_| hour(&mut store)).collect();
    assert!(store.counts().quiet);
    runs.sort_unstable();
    eprintln!("role traits quiet, fresh frame: {:?}", runs[HOURS / 2]);
}

/// A frame whose buffers hold the role store's CharacterRoleTraits table
/// publishes it again on a quiet hour without rewriting it (a planted marker
/// survives) while the hour carries every scientist row; a hooked writer
/// makes the next hour walk and rewrite the table.
#[test]
fn a_frame_publishes_its_held_role_traits_without_rewriting_them() {
    let world = World::new();
    let domain = Lookups::new();
    domain.watches.enable(world.characters.len());
    let census = world.census(&world.memory);
    let mut store = RoleStore::new();
    let mut frame = Captured::with_capacities(&Capacities::default());
    let hour = |frame: &mut Captured, store: &mut RoleStore| {
        // The pipeline grows every frame between hours.
        frame.grow(&mut Capacities::default());
        frame.reset();
        frame.capture_census_with(
            &world.memory,
            BASE,
            STATE,
            1,
            false,
            census.admitted(BASE, STATE),
            Some(store),
            &domain,
            &UNOBSERVED,
        );
        let [TableView::Complete(TableRows::CharacterRoleTraits(rows)), _] = frame.tables() else {
            panic!("complete character role traits");
        };
        rows
    };
    let first = hour(&mut frame, &mut store);
    for _ in 0..2 {
        assert_eq!(hour(&mut frame, &mut store), first);
        assert!(store.counts().quiet);
    }
    const MARK: u32 = 0x5eed;
    frame.traits.rows[0].character = MARK;
    let rows = hour(&mut frame, &mut store);
    assert!(store.counts().quiet);
    assert!(
        rows.iter().any(|row| row.character == MARK),
        "rows republished in place"
    );
    drop(domain.watches.notify(Some(world.characters[4]), 0));
    assert_eq!(hour(&mut frame, &mut store), first);
}

/// Operative links are carried while their leaders' certificates hold: an
/// unchanged hour reads no operative leader and publishes the walk's rows; a
/// notified leader is read again; a write no hook observed is carried.
#[test]
fn operative_links_are_carried_while_their_leaders_certificates_hold() {
    use crate::unit_leader_changes::Watches as LeaderWatches;
    const OPERATIVES: usize = 6;
    let (memory, leaders) = resolved_operatives(OPERATIVES);
    let world = World::of(memory);
    let domain = Lookups::new();
    domain.watches.enable(world.characters.len());
    let watches = LeaderWatches::new();
    watches.enable(64);
    let census = world.census(&world.memory);
    let objects: Vec<Range<usize>> = leaders
        .iter()
        .map(|&leader| leader..leader + 0x1100)
        .collect();
    let mut store = RoleStore::new();
    let hour = |store: &mut RoleStore, world: &World| {
        let watched = Watched {
            memory: &world.memory,
            roles: objects.clone(),
            hits: Cell::new(0),
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture_census_with(
            &watched,
            BASE,
            STATE,
            1,
            false,
            census.admitted(BASE, STATE),
            Some(store),
            &domain,
            &watches,
        );
        let [_, TableView::Complete(TableRows::CharacterOperatives(rows))] = captured.tables()
        else {
            panic!("complete character operatives");
        };
        (rows, watched.hits.get(), store.operative_counts())
    };
    let (first, reads, counts) = hour(&mut store, &world);
    assert!(reads > 0);
    assert_eq!(counts.read, OPERATIVES as u64);
    for _ in 0..2 {
        let (rows, reads, counts) = hour(&mut store, &world);
        assert_eq!(rows, first);
        assert_eq!(reads, 0, "an unchanged operative leader is not read");
        assert_eq!((counts.carried, counts.read), (OPERATIVES as u64, 0));
    }
    drop(watches.notify(leaders[2], 0));
    let (rows, reads, counts) = hour(&mut store, &world);
    assert_eq!(rows, first);
    assert!(reads > 0);
    assert_eq!((counts.carried, counts.read), (OPERATIVES as u64 - 1, 1));
    // A missed owner write on the fourth country's operative.
    let mut world = world;
    let owner = leaders[3] + cunit_leader::OWNER;
    world.memory.data[owner..owner + 4].copy_from_slice(&99_u32.to_le_bytes());
    let (stale, ..) = hour(&mut store, &world);
    assert_eq!(stale, first, "a write no hook saw is carried");
}

/// A link whose read refused a name is not held: once the name reads again,
/// with no leader write or notification, the next walk reads it afresh and
/// publishes it, as an uncarried walk would. Carried links keep their
/// assigned trait definitions.
#[test]
fn an_operative_link_whose_name_was_refused_is_read_again() {
    use crate::unit_leader_changes::Watches as LeaderWatches;
    use engine_native::cunit_leader_trait;
    const OPERATIVES: usize = 4;
    let (mut memory, leaders) = resolved_operatives(OPERATIVES);
    // Every operative holds one assigned trait of ordinal 7.
    let pointers = memory.data.len();
    let object = pointers + 0x10;
    memory.data.resize(object + cunit_leader_trait::SIZE, 0);
    memory.u64(pointers, object as u64);
    memory.u64(object, (BASE + cunit_leader_trait::VTABLE_RVA.0) as u64);
    let ordinal = object + cunit_leader_trait::ORDINAL;
    memory.data[ordinal..ordinal + 4].copy_from_slice(&7_u32.to_le_bytes());
    for &leader in &leaders {
        let header = leader + cunit_leader::TRAIT_DATA;
        memory.u64(header, pointers as u64);
        memory.data[header + 8..header + 16].copy_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0]);
    }
    let mut world = World::of(memory);
    let domain = Lookups::new();
    domain.watches.enable(world.characters.len());
    let watches = LeaderWatches::new();
    watches.enable(64);
    let census = world.census(&world.memory);
    let objects: Vec<Range<usize>> = leaders
        .iter()
        .map(|&leader| leader..leader + 0x1100)
        .collect();
    let hour = |store: &mut RoleStore, world: &World| {
        let watched = Watched {
            memory: &world.memory,
            roles: objects.clone(),
            hits: Cell::new(0),
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture_census_with(
            &watched,
            BASE,
            STATE,
            1,
            false,
            census.admitted(BASE, STATE),
            Some(store),
            &domain,
            &watches,
        );
        let [_, TableView::Complete(TableRows::CharacterOperatives(rows))] = captured.tables()
        else {
            panic!("complete character operatives");
        };
        (rows, watched.hits.get(), store.operative_counts())
    };
    let (clean, ..) = hour(&mut RoleStore::new(), &world);
    assert_eq!(clean.len(), OPERATIVES);
    assert!(clean.iter().all(|row| row.trait_definitions == [7]));
    // The second country's operative name header is unreadable for an hour.
    let length = leaders[1] + cunit_leader::NAME + 16;
    let readable: [u8; 8] = world.memory.data[length..length + 8].try_into().unwrap();
    world.memory.data[length..length + 8].copy_from_slice(&99_u64.to_le_bytes());
    let mut store = RoleStore::new();
    let (refused, _, counts) = hour(&mut store, &world);
    assert_ne!(refused, clean);
    assert_eq!(counts.read, OPERATIVES as u64);
    world.memory.data[length..length + 8].copy_from_slice(&readable);
    let (recovered, _, counts) = hour(&mut store, &world);
    assert_eq!(recovered, clean, "the refused name is read again");
    assert_eq!((counts.carried, counts.read), (OPERATIVES as u64 - 1, 1));
    let (carried, reads, counts) = hour(&mut store, &world);
    assert_eq!(carried, clean);
    assert_eq!(reads, 0);
    assert_eq!((counts.carried, counts.read), (OPERATIVES as u64, 0));
}
