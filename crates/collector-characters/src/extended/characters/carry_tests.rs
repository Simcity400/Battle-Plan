use super::carry::Carry;
use super::census_tests::{self, GAME_STATE, MODULE, World, capacities, context};
use super::leader_carry::{self, TraitCarry, UnitCarry};
use super::quiet::QuietRoster;
use super::*;
use collector_observers::character_changes::{Counting as Lookups, Watches};
use collector_observers::roster_changes::{Counts, Isolated, Roster as RosterDomain};
use collector_observers::unit_leader_changes::{
    Changes as LeaderChanges, Watches as LeaderWatches,
};
use std::cell::Cell;

struct Names<'a> {
    world: &'a World,
    characters: usize,
    hits: Cell<usize>,
}

impl Memory for Names<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let end = address.saturating_add(output.len());
        let first = World::character(0);
        let last = World::character(self.characters);
        if address < last && end > first {
            let offset = (address - first) % 0x100;
            let span = offset..offset + output.len();
            let carried =
                |range: std::ops::Range<usize>| span.start < range.end && span.end > range.start;
            if carried(ccharacter::NAME_TOKEN_ID..ccharacter::COUNTRY)
                || carried(ccharacter::GENDER..ccharacter::GENDER + 4)
            {
                self.hits.set(self.hits.get() + 1);
            }
        }
        self.world.copy(address, output)
    }
}

struct Hour {
    characters: TableView,
    reused: u64,
    read: u64,
    name_reads: usize,
    planned: bool,
}

fn hour(job: &mut CharactersJob, world: &World) -> Hour {
    let memory = Names {
        world,
        characters: world.characters,
        hits: Cell::new(0),
    };
    let report = copy_characters(&memory, context(PART_MASKS[0]), job, Roles::Gather);
    assert!(report.raw_ok && !report.plan_failed);
    let planned = job.log.planned_capture_succeeded();
    let mut captured = Captured::with_capacities(&capacities());
    captured.decode_characters_payload(MODULE, job, &mut [0; N]);
    captured.remember_templates(job);
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    let counts = job.carry.counts();
    Hour {
        characters: captured.tables()[0].clone(),
        reused: counts.reused,
        read: counts.read,
        name_reads: memory.hits.get(),
        planned,
    }
}

fn expected(world: &World) -> TableView {
    let mut captured = Captured::with_capacities(&capacities());
    let results = walk_family(
        world,
        MODULE,
        GAME_STATE,
        captured.family().select(PART_MASKS[0]),
        None,
        None,
    );
    captured.install_family(PART_MASKS[0], results);
    captured.tables()[0].clone()
}

fn world() -> World {
    World::new(200)
}

fn job(watches: &'static Watches) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    job.carry = Carry::with(watches);
    job
}

fn settle(job: &mut CharactersJob, world: &World) {
    for _ in 0..4 {
        hour(job, world);
    }
}

#[test]
fn quiet_hours_read_no_character_name_and_publish_the_roster_walk_rows() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    let world = world();
    WATCHES.enable(world.characters);
    let mut job = job(&WATCHES);
    let first = hour(&mut job, &world);
    assert_eq!(first.characters, expected(&world));
    assert!(first.name_reads > 0);
    settle(&mut job, &world);
    let quiet = hour(&mut job, &world);
    assert_eq!(quiet.characters, expected(&world));
    assert_eq!((quiet.reused, quiet.read), (world.characters as u64, 0));
    assert_eq!(quiet.name_reads, 0, "a quiet hour reads no carried span");
    assert!(
        quiet.planned,
        "the prepared program omits the carried names"
    );
}

#[test]
fn quiet_hours_recertify_the_carried_set_without_a_lookup() {
    let _globals = crate::extended::characters::test_globals();
    static DOMAIN: Lookups = Lookups::new();
    let mut world = world();
    let characters = world.characters as u64;
    DOMAIN.watches.enable(world.characters);
    let mut job = CharactersJob::new_part(0);
    job.carry = Carry::with(&DOMAIN);
    settle(&mut job, &world);
    hour(&mut job, &world);
    DOMAIN.lookups();
    for _ in 0..2 {
        let quiet = hour(&mut job, &world);
        assert_eq!(DOMAIN.lookups(), 0, "a quiet hour looks up no character");
        assert!(job.carry.counts().quiet);
        assert_eq!((quiet.reused, quiet.read), (characters, 0));
        assert_eq!(quiet.characters, expected(&world));
        assert!(quiet.planned);
    }
    world.patch(World::character(7) + ccharacter::NAME, b"Rena");
    drop(DOMAIN.watches.notify(
        Some(World::character(7)),
        engine_mutations::character_role_mutations::CHARACTER_NAME,
    ));
    let renamed = hour(&mut job, &world);
    assert!(DOMAIN.lookups() >= characters);
    assert!(!job.carry.counts().quiet);
    assert_eq!(renamed.characters, expected(&world));
    settle(&mut job, &world);
    DOMAIN.lookups();
    let quiet = hour(&mut job, &world);
    assert_eq!(DOMAIN.lookups(), 0);
    assert_eq!(quiet.characters, expected(&world));
    DOMAIN.watches.boundary(true);
    let reloaded = hour(&mut job, &world);
    assert!(DOMAIN.lookups() >= characters);
    assert_eq!(reloaded.reused, 0);
    assert_eq!(reloaded.characters, expected(&world));
}

#[test]
fn a_renamed_character_alone_is_read_and_a_new_world_reads_every_name() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    let mut job = job(&WATCHES);
    settle(&mut job, &world);
    world.patch(World::character(7) + ccharacter::NAME, b"Rena");
    drop(WATCHES.notify(
        Some(World::character(7)),
        engine_mutations::character_role_mutations::CHARACTER_NAME,
    ));
    let renamed = hour(&mut job, &world);
    assert_eq!(renamed.characters, expected(&world));
    assert_eq!(
        (renamed.reused, renamed.read),
        (world.characters as u64 - 1, 1)
    );
    WATCHES.boundary(true);
    let reloaded = hour(&mut job, &world);
    assert_eq!(reloaded.characters, expected(&world));
    assert_eq!(reloaded.reused, 0);
    assert!(reloaded.name_reads > 0);
}

struct Walk<'a> {
    world: &'a World,
    pages: Cell<usize>,
    identities: Cell<usize>,
    leaders: Cell<usize>,
    census: Cell<usize>,
}

impl Memory for Walk<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if [census_tests::HISTORICAL, census_tests::DYNAMIC]
            .iter()
            .any(|&region| address >> 32 == region >> 32)
        {
            self.pages.set(self.pages.get() + 1);
        }
        let first = World::character(0);
        let last = World::character(self.world.characters);
        if (first..last).contains(&address) && (address - first) % 0x100 < 0x10 {
            self.identities.set(self.identities.get() + 1);
        }
        if address >> 32 == census_tests::UNIT >> 32 {
            self.leaders.set(self.leaders.get() + 1);
        }
        if (first..last).contains(&address) {
            let offset = (address - first) % 0x100;
            let span = offset..offset + output.len();
            let gathered =
                |range: std::ops::Range<usize>| span.start < range.end && span.end > range.start;
            if gathered(ccharacter::COUNTRY..ccharacter::COUNTRY + 4)
                || gathered(ccharacter::LEADER_MAP_HEAD..ccharacter::OPERATIVE + 8)
            {
                self.census.set(self.census.get() + 1);
            }
        }
        self.world.copy(address, output)
    }
}

struct RosterHour {
    captured: Captured,
    portraits: Option<Vec<ccharacter_manager::PortraitIdentity>>,
    roles: Option<(Vec<census::Occurrence>, usize, u64)>,
    counts: Counts,
    pages: usize,
    identities: usize,
    leaders: usize,
    census: usize,
    planned: bool,
}

impl RosterHour {
    fn characters(&self) -> TableView {
        self.captured.tables()[0].clone()
    }

    fn units(&self) -> TableView {
        self.captured.tables()[3].clone()
    }
}

fn roster_hour(job: &mut CharactersJob, world: &World) -> RosterHour {
    let memory = Walk {
        world,
        pages: Cell::new(0),
        identities: Cell::new(0),
        leaders: Cell::new(0),
        census: Cell::new(0),
    };
    let report = copy_characters(&memory, context(PART_MASKS[0]), job, Roles::Gather);
    assert!(report.raw_ok && !report.plan_failed);
    let planned = job.log.planned_capture_succeeded();
    let portraits = job.portrait_census().and_then(|census| {
        census
            .rows(MODULE, GAME_STATE)
            .map(<[ccharacter_manager::PortraitIdentity]>::to_vec)
    });
    let roles = job.role_census().and_then(|census| {
        census.admitted(MODULE, GAME_STATE).map(|admitted| {
            (
                (0..admitted.len())
                    .map(|index| admitted.occurrence(index))
                    .collect(),
                admitted.historical,
                admitted.generation,
            )
        })
    });
    let mut captured = Captured::with_capacities(&capacities());
    captured.decode_characters_payload(MODULE, job, &mut [0; N]);
    captured.remember_templates(job);
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    RosterHour {
        captured,
        portraits,
        roles,
        counts: job.roster.counts(),
        pages: memory.pages.get(),
        identities: memory.identities.get(),
        leaders: memory.leaders.get(),
        census: memory.census.get(),
        planned,
    }
}

fn roster_job(watches: &'static Watches, domain: &'static dyn RosterDomain) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    job.carry = Carry::with(watches);
    job.roster = QuietRoster::with(domain);
    job
}

fn walked(world: &World) -> Captured {
    census_tests::typed(world, 0)
}

fn settle_roster(job: &mut CharactersJob, world: &World) {
    for _ in 0..4 {
        roster_hour(job, world);
    }
}

#[test]
fn quiet_roster_hours_publish_the_full_pass_without_walking_the_roster() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static NEVER: Isolated = Isolated::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut quiet = roster_job(&WATCHES, &DOMAIN);
    let mut full = roster_job(&WATCHES, &NEVER);
    let first = roster_hour(&mut quiet, &world);
    assert!(first.counts.read && !first.counts.quiet);
    assert!(first.pages > 0 && first.identities > 0);
    settle_roster(&mut quiet, &world);
    settle_roster(&mut full, &world);
    for hour in 0..3 {
        let read = roster_hour(&mut full, &world);
        let carried = roster_hour(&mut quiet, &world);
        assert!(read.counts.read && read.pages > 0 && read.identities > 0);
        assert!(carried.counts.quiet && !carried.counts.read, "hour {hour}");
        assert!(carried.planned, "hour {hour}: the unit-leader program");
        assert_eq!(
            (carried.pages, carried.identities),
            (0, 0),
            "a quiet hour reads no pointer page and no identity"
        );
        characters_test_kit::assert_matches_direct(&carried.captured, &read.captured);
        characters_test_kit::assert_matches_direct(&carried.captured, &walked(&world));
        assert!(carried.portraits.is_some());
        assert_eq!(carried.portraits, read.portraits, "portrait census");
        assert!(carried.roles.is_some());
        assert_eq!(carried.roles, read.roles, "role census and generation");
    }
    let [quiet_hours, read_hours, ..] = DOMAIN.counts();
    assert_eq!(read_hours, 1);
    assert!(quiet_hours >= 3);
    assert!(!DOMAIN.poisoned());
}

#[test]
fn a_settled_quiet_hour_republishes_its_characters_rows() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN);
    settle_roster(&mut job, &world);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    let report = copy_characters(&world, context(PART_MASKS[0]), &mut job, Roles::Gather);
    assert!(report.raw_ok && job.roster.quiet);
    assert!(job.published.matches(super::quiet::rows_key(MODULE, &job)));
    let version = job.carry.version;
    let carry = std::mem::replace(&mut job.carry, Carry::with(&WATCHES));
    job.carry.version = version;
    let mut captured = Captured::with_capacities(&capacities());
    captured.decode_characters_payload(MODULE, &job, &mut [0; N]);
    job.carry = carry;
    characters_test_kit::assert_matches_direct(&captured, &walked(&world));
}

#[test]
fn a_roster_write_reads_the_roster_again() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let mut world = World::new(201);
    world.roster = Some((0..200).collect());
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN);
    settle_roster(&mut job, &world);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    world.roster = Some((0..201).collect());
    DOMAIN.write();
    let pushed = roster_hour(&mut job, &world);
    assert!(pushed.counts.read && !pushed.counts.quiet);
    assert!(pushed.pages > 0 && pushed.identities > 0);
    characters_test_kit::assert_matches_direct(&pushed.captured, &walked(&world));
    let quiet = roster_hour(&mut job, &world);
    assert!(quiet.counts.quiet);
    assert_eq!((quiet.pages, quiet.identities), (0, 0));
    characters_test_kit::assert_matches_direct(&quiet.captured, &walked(&world));
    let running = DOMAIN.registry.notify(0);
    assert!(roster_hour(&mut job, &world).counts.read);
    drop(running);
    assert!(roster_hour(&mut job, &world).counts.read);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    DOMAIN.registry.boundary(true);
    let reloaded = roster_hour(&mut job, &world);
    assert!(reloaded.counts.read && reloaded.pages > 0);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    let full = CharactersCopyContext {
        force_full: true,
        ..context(PART_MASKS[0])
    };
    assert!(copy_characters(&world, full, &mut job, Roles::Gather).raw_ok);
    assert!(job.roster.counts().read);
}

#[test]
fn quiet_roster_hours_read_every_unit_leader_afresh() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN);
    settle_roster(&mut job, &world);
    let before = roster_hour(&mut job, &world);
    assert!(before.counts.quiet && before.planned);
    world.patch(
        World::unit(1).unwrap() + cunit_leader::EXILE_COUNTRY,
        &77_u32.to_le_bytes(),
    );
    let exiled = roster_hour(&mut job, &world);
    assert!(exiled.counts.quiet, "no roster writer ran");
    assert!(exiled.planned, "an exile country selects no read");
    assert_ne!(exiled.units(), before.units());
    characters_test_kit::assert_matches_direct(&exiled.captured, &walked(&world));
    assert_eq!(exiled.characters(), before.characters());
    world.patch(World::unit(5).unwrap() + cunit_leader::NAME, b"renamed!");
    let renamed = roster_hour(&mut job, &world);
    assert!(renamed.counts.quiet && !renamed.planned);
    assert_ne!(renamed.units(), exiled.units());
    characters_test_kit::assert_matches_direct(&renamed.captured, &walked(&world));
    assert_eq!(renamed.characters(), before.characters());
    let next = roster_hour(&mut job, &world);
    assert!(next.counts.quiet && next.planned);
    characters_test_kit::assert_matches_direct(&next.captured, &walked(&world));
    let generation = next.roles.as_ref().map(|roles| roles.2);
    world.patch(
        World::character(9) + ccharacter::COUNTRY,
        &77_u32.to_le_bytes(),
    );
    let moved = roster_hour(&mut job, &world);
    assert!(moved.counts.quiet && !moved.planned);
    assert_ne!(moved.roles.as_ref().map(|roles| roles.2), generation);
    characters_test_kit::assert_matches_direct(&moved.captured, &walked(&world));
    let after = roster_hour(&mut job, &world);
    assert!(after.counts.quiet && after.planned);
    characters_test_kit::assert_matches_direct(&after.captured, &walked(&world));
    assert!(!DOMAIN.poisoned());
}

#[test]
fn a_settled_quiet_roster_hour_reads_every_unit_leader_in_one_program() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: [Watches; 2] = [const { Watches::new() }; 2];
    static DOMAINS: [Isolated; 2] = [const { Isolated::new() }; 2];
    for (index, countries) in [40, 120].into_iter().enumerate() {
        let world = World::countries(countries);
        WATCHES[index].enable(world.characters);
        DOMAINS[index].enable();
        let mut job = roster_job(&WATCHES[index], &DOMAINS[index]);
        settle_roster(&mut job, &world);
        let units = (0..world.characters)
            .filter(|&character| World::unit(character).is_some())
            .count();
        let counting = census_tests::Counting::new(&world);
        let report =
            copy_characters(&counting, context(PART_MASKS[0]), &mut job, Roles::Gather);
        assert!(report.raw_ok && !report.plan_failed);
        assert!(job.roster.counts().quiet, "{countries} countries");
        assert!(job.log.planned_capture_succeeded(), "{countries} countries");
        assert_eq!(
            counting.calls.get(),
            2,
            "{countries} countries, {units} unit leaders: the role census and the program"
        );
        let mut captured = Captured::with_capacities(&capacities());
        captured.decode_characters_payload(MODULE, &job, &mut [0; N]);
        characters_test_kit::assert_matches_direct(&captured, &walked(&world));
    }
}

#[test]
fn a_unit_leader_read_failing_inside_the_program_fails_as_the_pass_does() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static UNPLANNED: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    UNPLANNED.enable();
    let mut planned = roster_job(&WATCHES, &DOMAIN);
    let mut unplanned = roster_job(&WATCHES, &UNPLANNED);
    settle_roster(&mut planned, &world);
    settle_roster(&mut unplanned, &world);
    world
        .faults
        .push(World::unit(9).unwrap() + cunit_leader::EXILE_COUNTRY);
    unplanned.log.invalidate_read_plan();
    unplanned.prepare_read_plans();
    let read = roster_hour(&mut unplanned, &world);
    let faulted = roster_hour(&mut planned, &world);
    assert!(read.counts.quiet && !read.planned);
    assert!(faulted.counts.quiet && !faulted.planned);
    assert!(!matches!(
        faulted.captured.outcomes()[3],
        Outcome::Rows { .. }
    ));
    characters_test_kit::assert_matches_direct(&faulted.captured, &read.captured);
    world.faults.clear();
    let traced = roster_hour(&mut planned, &world);
    assert!(traced.counts.quiet && !traced.planned);
    let repaired = roster_hour(&mut planned, &world);
    assert!(repaired.counts.quiet && repaired.planned);
    characters_test_kit::assert_matches_direct(&repaired.captured, &walked(&world));
}

#[test]
fn a_renamed_character_reads_the_roster_again() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN);
    settle_roster(&mut job, &world);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    world.patch(World::character(7) + ccharacter::NAME, b"Rena");
    drop(WATCHES.notify(
        Some(World::character(7)),
        engine_mutations::character_role_mutations::CHARACTER_NAME,
    ));
    let renamed = roster_hour(&mut job, &world);
    assert!(renamed.counts.read && !renamed.counts.quiet);
    assert!(renamed.pages > 0 && renamed.identities > 0);
    characters_test_kit::assert_matches_direct(&renamed.captured, &walked(&world));
    let quiet = roster_hour(&mut job, &world);
    assert!(quiet.counts.quiet);
    characters_test_kit::assert_matches_direct(&quiet.captured, &walked(&world));
    assert!(!DOMAIN.poisoned());
}

struct LeaderLookups {
    watches: LeaderWatches,
    lookups: std::sync::atomic::AtomicU64,
}

impl LeaderLookups {
    const fn new() -> Self {
        Self {
            watches: LeaderWatches::new(),
            lookups: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn take(&self) -> u64 {
        self.lookups.swap(0, std::sync::atomic::Ordering::Relaxed)
    }
}

impl LeaderChanges for LeaderLookups {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.watches.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        self.lookups
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.watches.sequence(leader)
    }
}

fn unit_job(
    watches: &'static Watches,
    domain: &'static dyn RosterDomain,
    leaders: &'static dyn LeaderChanges,
) -> CharactersJob {
    let mut job = roster_job(watches, domain);
    job.unit_carry = UnitCarry::with(leaders, leader_carry::UNITS);
    job
}

fn leaders(world: &World) -> u64 {
    (0..world.characters)
        .filter(|&index| World::unit(index).is_some())
        .count() as u64
}

#[test]
fn quiet_hours_read_no_unchanged_unit_leader() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderLookups = LeaderLookups::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.watches.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS);
    settle_roster(&mut job, &world);
    let held = roster_hour(&mut job, &world);
    assert!(held.counts.quiet);
    LEADERS.take();
    for _ in 0..2 {
        let carried = roster_hour(&mut job, &world);
        assert!(carried.counts.quiet);
        assert_eq!(carried.units(), walked(&world).tables()[3]);
        assert_eq!(carried.leaders, 0, "a quiet hour reads no unchanged leader");
        let counts = job.unit_carry.counts();
        assert_eq!((counts.reused, counts.read), (leaders(&world), 0));
        assert_eq!(LEADERS.take(), 0, "no writer ran: no lookup");
    }
    drop(LEADERS.watches.notify(World::unit(5).unwrap(), 0));
    let carried = roster_hour(&mut job, &world);
    assert_eq!(carried.units(), walked(&world).tables()[3]);
    assert_eq!(LEADERS.take(), leaders(&world));
    let counts = job.unit_carry.counts();
    assert_eq!((counts.reused, counts.read), (leaders(&world) - 1, 1));
}

#[test]
fn a_notified_leader_is_read_again() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS);
    settle_roster(&mut job, &world);
    roster_hour(&mut job, &world);
    let leader = World::unit(5).unwrap();
    world
        .patches
        .push((leader + cunit_leader::OWNER, 7_u32.to_le_bytes().to_vec()));
    drop(LEADERS.notify(leader, 13));
    let hour = roster_hour(&mut job, &world);
    assert_eq!(hour.units(), walked(&world).tables()[3]);
    let counts = job.unit_carry.counts();
    assert_eq!((counts.reused, counts.read), (leaders(&world) - 1, 1));
    assert!(hour.leaders > 0);
    assert!(!LEADERS.poisoned());
}

fn census_job(
    watches: &'static Watches,
    domain: &'static dyn RosterDomain,
    census: &'static collector_certificates::watch_registry::WatchRegistry,
) -> CharactersJob {
    let mut job = roster_job(watches, domain);
    job.census_carry = super::census::CensusCarry::with(census);
    job
}

#[test]
fn quiet_hours_keep_the_role_census_without_gathering_it() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static CENSUS: collector_certificates::watch_registry::WatchRegistry =
        collector_certificates::watch_registry::WatchRegistry::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    CENSUS.set_enabled(true);
    let mut job = census_job(&WATCHES, &DOMAIN, &CENSUS);
    settle_roster(&mut job, &world);
    let held = roster_hour(&mut job, &world);
    for _ in 0..2 {
        let kept = roster_hour(&mut job, &world);
        assert!(kept.counts.quiet);
        assert_eq!(kept.census, 0, "a kept census gathers no field");
        assert_eq!(kept.roles, held.roles);
        assert_eq!(kept.characters(), walked(&world).tables()[0]);
        assert_eq!(kept.units(), walked(&world).tables()[3]);
    }
    drop(CENSUS.notify(0));
    let gathered = roster_hour(&mut job, &world);
    assert_eq!(gathered.census, 2 * world.characters);
    assert_eq!(
        gathered.roles, held.roles,
        "unchanged bytes keep the generation"
    );
}

fn trait_hour(
    job: &mut CharactersJob,
    world: &World,
    loan: &census::RoleCensus,
) -> (TableView, usize) {
    let memory = Walk {
        world,
        pages: Cell::new(0),
        identities: Cell::new(0),
        leaders: Cell::new(0),
        census: Cell::new(0),
    };
    let report = copy_characters(&memory, context(PART_MASKS[3]), job, Roles::Borrow(loan));
    assert!(report.raw_ok && !report.plan_failed);
    let mut captured = Captured::with_capacities(&capacities());
    captured.decode_characters_payload(MODULE, job, &mut [0; N]);
    captured.remember_templates(job);
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    (captured.tables()[4].clone(), memory.leaders.get())
}

fn trait_job(leaders: &'static LeaderWatches, world: &World) -> (CharactersJob, CharactersJob) {
    let mut job = CharactersJob::new_part(3);
    job.trait_carry = TraitCarry::with(leaders, leader_carry::TRAITS);
    (job, census_tests::roster(world))
}

fn traits_walked(world: &World) -> TableView {
    census_tests::typed(world, 3).tables()[4].clone()
}

fn swap_trait(world: &mut World, index: usize, other: usize) {
    let mut definition = [0; 8];
    assert!(world.copy(census_tests::TRAITS + other * 0x40, &mut definition));
    world
        .patches
        .push((census_tests::TRAITS + index * 0x40, definition.to_vec()));
}

#[test]
fn unchanged_leaders_carry_their_traits_without_reading_them() {
    let _globals = crate::extended::characters::test_globals();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    LEADERS.enable(world.characters);
    let (mut job, owner) = trait_job(&LEADERS, &world);
    let loan = owner.role_census().unwrap();
    let (first, read) = trait_hour(&mut job, &world, &loan);
    assert_eq!(first, traits_walked(&world));
    assert!(read > 0);
    for _ in 0..2 {
        let (carried, read) = trait_hour(&mut job, &world, &loan);
        assert_eq!(carried, traits_walked(&world));
        assert_eq!(read, 0, "an unchanged leader is not read");
        let counts = job.trait_carry.counts();
        assert_eq!((counts.reused, counts.read), (leaders(&world), 0));
    }
    drop(LEADERS.notify(World::unit(5).unwrap(), 0));
    let (renewed, read) = trait_hour(&mut job, &world, &loan);
    assert_eq!(renewed, traits_walked(&world));
    assert!(read > 0);
    assert_eq!(job.trait_carry.counts().read, 1);
    swap_trait(&mut world, 5, 9);
    drop(LEADERS.notify(World::unit(5).unwrap(), 0));
    let (changed, _) = trait_hour(&mut job, &world, &loan);
    assert_eq!(changed, traits_walked(&world));
    let (carried, read) = trait_hour(&mut job, &world, &loan);
    assert_eq!((carried, read), (traits_walked(&world), 0));
    assert!(!LEADERS.poisoned());
}

fn roster_hour_into(
    job: &mut CharactersJob,
    world: &World,
    frame: &mut Captured,
) -> (TableView, TableView) {
    let report = copy_characters(world, context(PART_MASKS[0]), job, Roles::Gather);
    assert!(report.raw_ok && !report.plan_failed);
    frame.grow(&mut capacities());
    frame.reset();
    frame.decode_characters_payload(MODULE, job, &mut [0; N]);
    frame.remember_templates(job);
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    let tables = frame.tables();
    (tables[0].clone(), tables[3].clone())
}

#[test]
fn fresh_leader_reads_grow_for_later_training() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    let first = World::unit(5).unwrap();
    world.patch(
        first + cunit_leader::TRAIT_EXPERIENCE_COUNT,
        &1_u32.to_le_bytes(),
    );
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS);
    settle_roster(&mut job, &world);
    let mut frame = Captured::with_capacities(&capacities());
    for _ in 0..2 {
        roster_hour_into(&mut job, &world, &mut frame);
    }
    assert!(frame.held.units.is_some());
    let expected = walked(&world).tables();
    assert_eq!(frame.tables()[3], expected[3]);
    const MARK: u32 = 0x5eed;
    frame.units.rows.rows[0].native.owner = MARK;
    let held = frame.tables()[3].clone();
    for _ in 0..2 {
        drop(LEADERS.notify(first, 0));
        drop(LEADERS.notify(World::unit(29).unwrap(), 0));
        let (_, units) = roster_hour_into(&mut job, &world, &mut frame);
        assert_eq!(units, held, "rows republished in place");
        assert_eq!(frame.tables()[5], expected[5]);
        assert_eq!(job.unit_carry.counts().read, 2);
    }
    assert!(!LEADERS.poisoned());
}

#[test]
fn quiet_leader_comparison_preserves_training_refusals() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS);
    settle_roster(&mut job, &world);
    let mut frame = Captured::with_capacities(&capacities());
    for _ in 0..2 {
        roster_hour_into(&mut job, &world, &mut frame);
    }
    let leader = World::unit(5).unwrap();
    world.patch(
        leader + cunit_leader::TRAIT_EXPERIENCE_FACTORS + engine_native_memory::cpdx_array::COUNT,
        &3_u32.to_le_bytes(),
    );
    world.patch(leader + cunit_leader::EXPERIENCE, &i64::MIN.to_le_bytes());
    drop(LEADERS.notify(leader, 30));
    roster_hour_into(&mut job, &world, &mut frame);
    let expected = walked(&world).tables();
    assert_eq!(frame.tables()[3], expected[3]);
    assert_eq!(frame.tables()[5], expected[5]);
    assert!(matches!(frame.outcomes()[5], Outcome::Unavailable { .. }));
    assert_eq!(job.unit_carry.counts().read, 1);
    world.patch(
        leader + cunit_leader::TRAIT_EXPERIENCE_FACTORS + engine_native_memory::cpdx_array::COUNT,
        &2_u32.to_le_bytes(),
    );
    roster_hour_into(&mut job, &world, &mut frame);
    assert_eq!(frame.tables()[5], walked(&world).tables()[5]);
    assert!(matches!(frame.outcomes()[5], Outcome::Rows { .. }));
    assert_eq!(job.unit_carry.counts().read, 1);
    assert!(!LEADERS.poisoned());
}

#[test]
fn leader_carry_rejects_mismatched_trait_spans() {
    let _globals = crate::extended::characters::test_globals();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    LEADERS.enable(1);
    let mut carry = TraitCarry::with(&LEADERS, leader_carry::TRAITS);
    let row = cunit_leader::TraitRow {
        leader_id: 7,
        token_len: 1,
        ..Default::default()
    };
    let certify = |carry: &mut TraitCarry| {
        carry.begin_hour();
        drop(LEADERS.notify(0x1000, 0));
        assert!(!carry.certify(1, 1, || std::iter::once((0, 0x1000, 7))));
    };
    certify(&mut carry);
    carry.rebuild(Outcome::Rows { len: 1 }, &[row], b"x", |_| Some((1, 7)));
    certify(&mut carry);
    assert!(carry.held_equals(0, &[row], b"x"));
    for (token_start, token_len) in [(1, 1), (usize::MAX, 1), (0, usize::MAX)] {
        let invalid = cunit_leader::TraitRow {
            token_start,
            token_len,
            ..row
        };
        assert!(!carry.held_equals(0, &[invalid], b"x"));
    }
    let invalid = cunit_leader::TraitRow {
        token_start: 1,
        ..row
    };
    carry.rebuild(Outcome::Rows { len: 1 }, &[invalid], b"x", |_| Some((1, 7)));
    assert!(!carry.held_equals(0, &[row], b"x"));
}

#[test]
fn forced_full_refresh_publishes_current_training_without_a_stale_carry_key() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS);
    settle_roster(&mut job, &world);
    let mut frame = Captured::with_capacities(&capacities());
    roster_hour_into(&mut job, &world, &mut frame);
    roster_hour_into(&mut job, &world, &mut frame);
    let mut cache = None;
    let mut output = match frame.training_input(&mut cache) {
        TableInput::SharedComplete(rows) => rows,
        _ => panic!("complete training baseline"),
    };
    assert!(cache.is_some(), "the baseline must exercise the carry memo");
    let leader = World::unit(5).unwrap();
    let progress = engine_native_memory::read::<u64>(&world, leader + cunit_leader::TRAIT_EXPERIENCE_DATA)
        .unwrap() as usize;
    for (list, address, site, value) in [
        (0, progress + 8, 26, 1_234_567_i64),
        (1, World::factors(5) + 8, 30, -765_432_i64),
    ] {
        let generation = frame.held.units.unwrap().0.0;
        let scalar = frame.tables()[3].clone();
        world.patch(address, &value.to_le_bytes());
        drop(LEADERS.notify(leader, site));
        let mut ctx = context(PART_MASKS[0]);
        ctx.force_full = true;
        let report = copy_characters(&world, ctx, &mut job, Roles::Gather);
        assert!(report.raw_ok && !report.plan_failed);
        assert!(!job.roster.quiet);
        assert_eq!(quiet::census_generation(&job), generation);
        frame.grow(&mut capacities());
        frame.reset();
        frame.decode_characters_payload(MODULE, &job, &mut [0; N]);
        frame.remember_templates(&mut job);
        assert_eq!(frame.tables()[3], scalar, "only training changed");
        let expected = frame.tables()[5].clone();
        let TableInput::SharedComplete(current) = frame.training_input(&mut cache) else {
            panic!("complete refreshed training");
        };
        assert!(!Arc::ptr_eq(&output, &current));
        assert_eq!(TableView::Complete((*current).clone()), expected);
        let TableRows::UnitLeaderTraitExperience(rows) = current.as_ref() else {
            panic!("training rows");
        };
        assert!(
            rows.iter()
                .any(|row| row.list == list && row.experience == value)
        );
        if job.grow_for_next() {
            job.grow();
        }
        job.prepare_read_plans();
        job.clear();
        roster_hour_into(&mut job, &world, &mut frame);
        let held = roster_hour(&mut job, &world);
        assert_eq!(held.leaders, 0, "quiet recovery still carries every leader");
        assert_eq!(held.captured.tables()[5], expected);
        output = match frame.training_input(&mut cache) {
            TableInput::SharedComplete(rows) => rows,
            _ => panic!("complete renewed training"),
        };
    }
}

#[test]
fn experience_mutations_refresh_one_leader_and_carry_complete_training() {
    let _globals = crate::extended::characters::test_globals();
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS);
    settle_roster(&mut job, &world);
    let before = roster_hour(&mut job, &world);
    let held = roster_hour(&mut job, &world);
    assert!(
        matches!(held.captured.outcomes()[5], Outcome::Rows { .. }),
        "training: {:?}; first training refusal: {:?}",
        held.captured.outcomes()[5],
        held.captured
            .units
            .rows
            .rows
            .iter()
            .find_map(|row| row.training_reason)
    );
    assert_eq!(
        held.leaders, 0,
        "unchanged XP and training perform no native leader read"
    );
    assert_eq!(held.captured.tables()[5], before.captured.tables()[5]);
    assert!(
        matches!(&held.captured.tables()[5], TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) if !rows.is_empty())
    );
    let leader = World::unit(5).unwrap();
    for site in 23..32 {
        let value = i64::MAX - site as i64;
        world.patch(leader + cunit_leader::EXPERIENCE, &value.to_le_bytes());
        if site >= 30 {
            world.patch(World::factors(5) + 8, &value.to_le_bytes());
        }
        drop(LEADERS.notify(leader, site));
        let changed = roster_hour(&mut job, &world);
        assert_eq!(changed.units(), walked(&world).tables()[3]);
        assert_eq!(changed.captured.tables()[5], walked(&world).tables()[5]);
        assert_eq!(
            (job.unit_carry.counts().reused, job.unit_carry.counts().read),
            (leaders(&world) - 1, 1)
        );
        assert!(
            matches!(&changed.units(), TableView::Complete(TableRows::UnitLeaders(rows)) if rows.iter().any(|row| row.experience == value))
        );
        assert_eq!(roster_hour(&mut job, &world).leaders, 0);
    }
}
