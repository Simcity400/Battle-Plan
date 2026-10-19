//! Characters rows carried by the roster job ([`super::carry::Carry`]) over
//! the census test world: hours run copy, decode, template and carry rebuild,
//! then recycle, as the pipeline does. The quiet roster hours
//! ([`super::quiet`]) run the same hours with a roster domain of their own,
//! and carry UnitLeaders rows with a leader domain of their own
//! ([`super::unit_carry`]).

use super::carry::Carry;
use super::census_tests::{self, GAME_STATE, MODULE, World, capacities, context};
use super::leader_carry::{self, TraitCarry, UnitCarry};
use super::quiet::QuietRoster;
use super::*;
use crate::character_changes::{Counting as Lookups, Watches};
use crate::roster_changes::{Counts, Isolated, Roster as RosterDomain};
use crate::sampled_verify::Check;
use crate::unit_leader_changes::{Changes as LeaderChanges, Watches as LeaderWatches};
use std::cell::Cell;

/// Counts reads touching a carried span (gender) of any character object.
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
    verified: u64,
    mismatches: u64,
    name_reads: usize,
    /// The roster job's prepared program served the hour.
    planned: bool,
}

fn hour(job: &mut CharactersJob, world: &World) -> Hour {
    let memory = Names {
        world,
        characters: world.characters,
        hits: Cell::new(0),
    };
    let report = copy_characters_growing(&memory, context(PART_MASKS[0]), job, Roles::Gather);
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
        verified: counts.verified,
        mismatches: counts.mismatches,
        name_reads: memory.hits.get(),
        planned,
    }
}

/// The typed roster walk's Characters table.
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

fn job(watches: &'static Watches, check: Check) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    job.carry = Carry::with(watches, Some(check));
    job
}

/// Runs hours until the carried set holds every character and the prepared
/// program omits their names.
fn settle(job: &mut CharactersJob, world: &World) {
    for _ in 0..4 {
        hour(job, world);
    }
}

#[test]
fn quiet_hours_read_no_character_name_and_publish_the_roster_walk_rows() {
    static WATCHES: Watches = Watches::new();
    let world = world();
    WATCHES.enable(world.characters);
    let mut job = job(&WATCHES, Check::Nothing);
    // The first hour reads and holds every row; later hours carry them.
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
    static DOMAIN: Lookups = Lookups::new();
    let mut world = world();
    let characters = world.characters as u64;
    DOMAIN.watches.enable(world.characters);
    let mut job = CharactersJob::new_part(0);
    job.carry = Carry::with(&DOMAIN, Some(Check::Nothing));
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
    // An observed rename: every held character is certified again and the
    // renamed one read; quiet hours follow once the set settles.
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
    // A load, gap or recovery certifies every character afresh.
    DOMAIN.watches.boundary(true);
    let reloaded = hour(&mut job, &world);
    assert!(DOMAIN.lookups() >= characters);
    assert_eq!(reloaded.reused, 0);
    assert_eq!(reloaded.characters, expected(&world));
}

#[test]
fn a_renamed_character_alone_is_read_and_a_new_world_reads_every_name() {
    static WATCHES: Watches = Watches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    let mut job = job(&WATCHES, Check::Nothing);
    settle(&mut job, &world);
    // set_character_name on character 7, observed by its hooked setter.
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
    // A load, gap or recovery: nothing carries across it.
    WATCHES.boundary(true);
    let reloaded = hour(&mut job, &world);
    assert_eq!(reloaded.characters, expected(&world));
    assert_eq!(reloaded.reused, 0);
    assert!(reloaded.name_reads > 0);
}

#[test]
fn verify_mode_reads_every_name_and_poisons_on_an_unobserved_rename() {
    static WATCHES: Watches = Watches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    let mut job = job(&WATCHES, Check::Every);
    settle(&mut job, &world);
    let verified = hour(&mut job, &world);
    assert_eq!((verified.reused, verified.mismatches), (0, 0));
    assert!(verified.name_reads > 0);
    assert!(!WATCHES.poisoned());
    // A rename no hook observed.
    world.patch(World::character(9) + ccharacter::NAME, b"Odd!");
    let caught = hour(&mut job, &world);
    assert_eq!(caught.mismatches, 1);
    assert!(WATCHES.poisoned());
    assert_eq!(caught.characters, expected(&world));
}

#[test]
fn sampled_hours_cover_every_carried_name_and_catch_an_unobserved_rename_within_one_rotation() {
    static WATCHES: Watches = Watches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    let mut job = job(&WATCHES, Check::Nothing);
    settle(&mut job, &world);
    // Any 24 consecutive hours read each carried character once, outside
    // the prepared program, which still serves every hour.
    let mut compared = 0;
    for at in 0..24 {
        job.carry.set_check(Check::Slice { hour: at });
        let quiet = hour(&mut job, &world);
        assert_eq!(quiet.characters, expected(&world));
        assert_eq!((quiet.read, quiet.mismatches), (0, 0));
        assert!(quiet.planned, "hour {at}");
        compared += quiet.verified;
    }
    assert_eq!(compared, world.characters as u64);
    // A rename no hook observed.
    world.patch(World::character(9) + ccharacter::NAME, b"Odd!");
    let mut id = [0; 4];
    assert!(world.copy(World::character(9) + ccharacter::ID, &mut id));
    let id = u32::from_le_bytes(id);
    let mut caught = None;
    for at in 0..24 {
        job.carry.set_check(Check::Slice { hour: at });
        let sampled = hour(&mut job, &world);
        if sampled.mismatches != 0 {
            caught = Some((at, sampled));
            break;
        }
        assert_ne!(sampled.characters, expected(&world), "hour {at} carries");
    }
    let (at, caught) = caught.expect("caught within one rotation");
    assert_eq!(at as u32, id % 24);
    assert_eq!(caught.mismatches, 1);
    assert!(WATCHES.poisoned());
    assert_eq!(
        caught.characters,
        expected(&world),
        "the hour reads every name afresh"
    );
}

/// Counts the reads a quiet roster hour does not make: the roster's pointer pages and each
/// character's identity span (vtable, reference type and id).
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

/// One roster-job hour: what it published and what it read.
struct RosterHour {
    captured: Captured,
    portraits: Option<Vec<ccharacter_manager::PortraitIdentity>>,
    /// Occurrences, historical length and generation.
    roles: Option<(Vec<census::Occurrence>, usize, u64)>,
    counts: Counts,
    pages: usize,
    identities: usize,
    /// Reads of unit leader objects.
    leaders: usize,
    /// Reads of the role census's gathered fields.
    census: usize,
    /// The job's prepared program served the hour's reads.
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

/// A role hour of the roster job, run as `hour` runs it, keeping the
/// censuses it loans.
fn roster_hour(job: &mut CharactersJob, world: &World) -> RosterHour {
    let memory = Walk {
        world,
        pages: Cell::new(0),
        identities: Cell::new(0),
        leaders: Cell::new(0),
        census: Cell::new(0),
    };
    let report = copy_characters_growing(&memory, context(PART_MASKS[0]), job, Roles::Gather);
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

fn roster_job(
    watches: &'static Watches,
    domain: &'static dyn RosterDomain,
    check: Check,
) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    job.carry = Carry::with(watches, Some(Check::Nothing));
    job.roster = QuietRoster::with(domain, Some(check));
    job
}

/// The typed roster walk's Characters and UnitLeaders over `world`.
fn walked(world: &World) -> Captured {
    census_tests::typed(world, 0)
}

/// Runs hours until the roster job carries every character.
fn settle_roster(job: &mut CharactersJob, world: &World) {
    for _ in 0..4 {
        roster_hour(job, world);
    }
}

/// A quiet roster hour publishes exactly what the full pass publishes over
/// the same memory (Characters and UnitLeaders rows and arenas, the portrait
/// census and the role census with its generation) and reads no roster
/// pointer page and no character identity, where the full pass reads both.
#[test]
fn quiet_roster_hours_publish_the_full_pass_without_walking_the_roster() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static NEVER: Isolated = Isolated::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut quiet = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
    // A job whose domain is never observed always runs the full pass.
    let mut full = roster_job(&WATCHES, &NEVER, Check::Nothing);
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

/// A quiet roster hour whose role census and carry are those of the job's
/// latest quiet decode publishes that decode's Characters rows again,
/// without rebuilding them from the carry: here the carry is emptied under
/// the same version, so a rebuild would find no held row.
#[test]
fn a_settled_quiet_hour_republishes_its_characters_rows() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
    settle_roster(&mut job, &world);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    let report = copy_characters_growing(&world, context(PART_MASKS[0]), &mut job, Roles::Gather);
    assert!(report.raw_ok && job.roster.quiet);
    assert!(job.published.matches(super::quiet::rows_key(MODULE, &job)));
    let version = job.carry.version;
    let carry = std::mem::replace(&mut job.carry, Carry::with(&WATCHES, Some(Check::Nothing)));
    job.carry.version = version;
    let mut captured = Captured::with_capacities(&capacities());
    captured.decode_characters_payload(MODULE, &job, &mut [0; N]);
    job.carry = carry;
    characters_test_kit::assert_matches_direct(&captured, &walked(&world));
}

/// A roster write (an appended character here), a writer running at the
/// hour, a load or a full hour reads the roster again; the hour after is
/// quiet.
#[test]
fn a_roster_write_reads_the_roster_again() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let mut world = World::new(201);
    world.roster = Some((0..200).collect());
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
    settle_roster(&mut job, &world);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    // Character creation and registration appends character 200 to the roster.
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
    // A writer running at the hour gives no token.
    let running = DOMAIN.registry.notify(0);
    assert!(roster_hour(&mut job, &world).counts.read);
    drop(running);
    assert!(roster_hour(&mut job, &world).counts.read);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    // A load, gap or recovery starts an epoch.
    DOMAIN.registry.boundary(true);
    let reloaded = roster_hour(&mut job, &world);
    assert!(reloaded.counts.read && reloaded.pages > 0);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    // A full hour reads the roster too.
    let full = CharactersCopyContext {
        force_full: true,
        ..context(PART_MASKS[0])
    };
    assert!(copy_characters_growing(&world, full, &mut job, Roles::Gather).raw_ok);
    assert!(job.roster.counts().read);
}

/// Unit leaders change at any hour without a notification: a quiet hour
/// still reads every one afresh, through its prepared program. A changed
/// value no read depends on (an exile country) is published by a completed
/// program; a changed byte that selects later reads (a name header, whose
/// length and storage select the payload reads) stops the program there and
/// the pass continues from that read; a changed role census (a character's
/// country) changes the census's generation, so the pass reads one by one
/// and prepares the next program. Every hour equals the full pass.
#[test]
fn quiet_roster_hours_read_every_unit_leader_afresh() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
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
    // Character 9 moves to country 77: the role census differs.
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

/// A settled quiet roster hour gathers its role census and runs its
/// unit-leader program: two guarded calls at 1,500 and 5,000 countries,
/// however many unit leaders there are, where reading the leaders one by
/// one takes several calls each. It publishes the full pass's tables.
#[test]
fn a_settled_quiet_roster_hour_reads_every_unit_leader_in_one_program() {
    static WATCHES: [Watches; 2] = [const { Watches::new() }; 2];
    static DOMAINS: [Isolated; 2] = [const { Isolated::new() }; 2];
    for (index, countries) in [1_500, 5_000].into_iter().enumerate() {
        let world = World::countries(countries);
        WATCHES[index].enable(world.characters);
        DOMAINS[index].enable();
        let mut job = roster_job(&WATCHES[index], &DOMAINS[index], Check::Nothing);
        settle_roster(&mut job, &world);
        let units = (0..world.characters)
            .filter(|&character| World::unit(character).is_some())
            .count();
        let counting = census_tests::Counting::new(&world);
        let report =
            copy_characters_growing(&counting, context(PART_MASKS[0]), &mut job, Roles::Gather);
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

/// A unit-leader read that fails inside the program fails the hour at that
/// read, exactly as a quiet hour without a program reads and fails it: both
/// publish the same tables. The failed hour prepares no program; once the
/// fault is gone, the next hour prepares one and the hour after runs it.
#[test]
fn a_unit_leader_read_failing_inside_the_program_fails_as_the_pass_does() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static UNPLANNED: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    UNPLANNED.enable();
    let mut planned = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
    let mut unplanned = roster_job(&WATCHES, &UNPLANNED, Check::Nothing);
    settle_roster(&mut planned, &world);
    settle_roster(&mut unplanned, &world);
    world
        .faults
        .push(World::unit(9).unwrap() + cunit_leader::EXILE_COUNTRY);
    // Without its program, the second job reads the leaders one by one.
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

/// A roster change no hook observed (two slots swapped in memory) is caught
/// by the sampled slice within one rotation, or by verify mode at once; it
/// poisons the domain and that hour reads afresh.
#[test]
fn an_unobserved_roster_change_is_caught_by_the_slice_or_verify_mode() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static VERIFIED: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    VERIFIED.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
    let mut verify = roster_job(&WATCHES, &VERIFIED, Check::Every);
    settle_roster(&mut job, &world);
    settle_roster(&mut verify, &world);
    // Any 24 consecutive quiet hours compare every slot and id once, beside
    // both headers each hour.
    let mut compared = 0;
    for at in 0..24 {
        job.roster.set_check(Check::Slice { hour: at });
        let quiet = roster_hour(&mut job, &world);
        assert!(quiet.counts.quiet, "hour {at}");
        assert_eq!(quiet.counts.mismatches, 0);
        compared += quiet.counts.verified - 1;
        characters_test_kit::assert_matches_direct(&quiet.captured, &walked(&world));
    }
    assert_eq!(compared, world.characters as u64);
    // Verify mode reads every hour and compares the whole census.
    let checked = roster_hour(&mut verify, &world);
    assert!(checked.counts.read && !checked.counts.quiet && checked.pages > 0);
    assert_eq!(
        (checked.counts.verified, checked.counts.mismatches),
        (world.characters as u64, 0)
    );
    assert!(!VERIFIED.poisoned());
    // Slots 10 and 11 swapped, with no roster write.
    let mut swapped: Vec<usize> = (0..world.characters).collect();
    swapped.swap(10, 11);
    world.roster = Some(swapped);
    let id = |index: usize| {
        let mut id = [0; 4];
        assert!(world.copy(World::character(index) + ccharacter::ID, &mut id));
        u64::from(u32::from_le_bytes(id))
    };
    let first = (id(10) % 24).min(id(11) % 24) as i32;
    let fresh = walked(&world);
    let mut caught = None;
    for at in 0..24 {
        job.roster.set_check(Check::Slice { hour: at });
        let hour = roster_hour(&mut job, &world);
        if hour.counts.mismatches != 0 {
            caught = Some((at, hour));
            break;
        }
        assert!(hour.counts.quiet);
        assert_ne!(hour.characters(), fresh.tables()[0], "hour {at} carries");
    }
    let (at, caught) = caught.expect("caught within one rotation");
    assert_eq!(at, first);
    assert!(DOMAIN.poisoned());
    assert!(caught.counts.read && !caught.counts.quiet && caught.pages > 0);
    characters_test_kit::assert_matches_direct(&caught.captured, &fresh);
    // Every later hour reads afresh.
    let after = roster_hour(&mut job, &world);
    assert!(after.counts.read && after.pages > 0);
    // Verify mode catches it the first hour.
    let verified = roster_hour(&mut verify, &world);
    assert_eq!(verified.counts.mismatches, 1);
    assert!(VERIFIED.poisoned());
    characters_test_kit::assert_matches_direct(&verified.captured, &fresh);
}

/// A renamed character's certificate is revoked: that hour reads the roster
/// and the name afresh; the next is quiet again.
#[test]
fn a_renamed_character_reads_the_roster_again() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    let mut job = roster_job(&WATCHES, &DOMAIN, Check::Nothing);
    settle_roster(&mut job, &world);
    assert!(roster_hour(&mut job, &world).counts.quiet);
    // set_character_name on character 7, observed by its hooked setter.
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

/// A leader domain that counts its per-leader lookups.
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
    fn mismatch(&self) {
        self.watches.mismatch();
    }
}

/// A roster job whose quiet hours carry UnitLeaders rows under `leaders`.
fn unit_job(
    watches: &'static Watches,
    domain: &'static dyn RosterDomain,
    leaders: &'static dyn LeaderChanges,
    check: Check,
) -> CharactersJob {
    let mut job = roster_job(watches, domain, Check::Nothing);
    job.unit_carry = UnitCarry::with(leaders, leader_carry::UNITS, Some(check));
    job
}

/// The world's unit leaders: every fourth character's.
fn leaders(world: &World) -> u64 {
    (0..world.characters)
        .filter(|&index| World::unit(index).is_some())
        .count() as u64
}

/// A quiet hour after one that held every leader reads no unit leader and
/// publishes the full pass's UnitLeaders rows; while no hooked writer runs
/// it looks up no leader's certificate either. A writer's notification
/// makes the next hour look up every one.
#[test]
fn quiet_hours_read_no_unchanged_unit_leader() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderLookups = LeaderLookups::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.watches.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS, Check::Nothing);
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

/// A hooked writer's leader is read the next quiet hour, which publishes its
/// new row; the others stay carried.
#[test]
fn a_notified_leader_is_read_again() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS, Check::Nothing);
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

/// A write no hook observed keeps a carried row stale until the leader's
/// slice reads it: that hour publishes the fresh row and poisons reuse, so
/// the next quiet hour reads every leader.
#[test]
fn the_slice_catches_a_missed_leader_write() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS, Check::Nothing);
    settle_roster(&mut job, &world);
    roster_hour(&mut job, &world);
    let leader = World::unit(9).unwrap();
    world
        .patches
        .push((leader + cunit_leader::OWNER, 7_u32.to_le_bytes().to_vec()));
    let stale = roster_hour(&mut job, &world);
    assert_ne!(stale.units(), walked(&world).tables()[3]);
    // The hour of the day whose slice holds the leader's id.
    let id: u32 = 2_000_000 + 9;
    job.unit_carry.set_check(Check::Slice {
        hour: (id % 24) as i32,
    });
    let caught = roster_hour(&mut job, &world);
    assert_eq!(caught.units(), walked(&world).tables()[3]);
    let counts = job.unit_carry.counts();
    assert_eq!((counts.verified, counts.mismatches), (counts.read, 1));
    assert!(LEADERS.poisoned());
    roster_hour(&mut job, &world);
    let counts = job.unit_carry.counts();
    assert_eq!((counts.reused, counts.read), (0, leaders(&world)));
}

/// A roster job whose role census is kept under `census`.
fn census_job(
    watches: &'static Watches,
    domain: &'static dyn RosterDomain,
    census: &'static crate::watch_registry::WatchRegistry,
    check: Check,
) -> CharactersJob {
    let mut job = roster_job(watches, domain, Check::Nothing);
    job.census_carry = super::census::CensusCarry::with(census, Some(check));
    job
}

/// Once a gather admitted the census under the census domain's token, a
/// role hour with the same token and occurrences keeps it, gathering no
/// field, with its generation, and publishes the full pass's rows. A census
/// writer makes the next hour gather it again.
#[test]
fn quiet_hours_keep_the_role_census_without_gathering_it() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static CENSUS: crate::watch_registry::WatchRegistry =
        crate::watch_registry::WatchRegistry::new();
    let world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    CENSUS.set_enabled(true);
    let mut job = census_job(&WATCHES, &DOMAIN, &CENSUS, Check::Nothing);
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

/// A write no census hook observed (a character's country here) keeps a
/// stale census until the character's slice gathers it: that hour gathers
/// the whole census, publishes its rows and poisons the carry.
#[test]
fn the_slice_catches_a_missed_census_write() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static CENSUS: crate::watch_registry::WatchRegistry =
        crate::watch_registry::WatchRegistry::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    CENSUS.set_enabled(true);
    let mut job = census_job(&WATCHES, &DOMAIN, &CENSUS, Check::Nothing);
    settle_roster(&mut job, &world);
    roster_hour(&mut job, &world);
    world.patches.push((
        World::character(40) + ccharacter::COUNTRY,
        99_u32.to_le_bytes().to_vec(),
    ));
    let stale = roster_hour(&mut job, &world);
    assert_ne!(stale.characters(), walked(&world).tables()[0]);
    let id: u32 = 1_000_000 + 40;
    job.census_carry.set_check(Check::Slice {
        hour: (id % 24) as i32,
    });
    let caught = roster_hour(&mut job, &world);
    assert_eq!(caught.characters(), walked(&world).tables()[0]);
    assert!(CENSUS.poisoned());
    let next = roster_hour(&mut job, &world);
    assert_eq!(next.census, 2 * world.characters);
}

/// The traits job of a role hour over `loan`, run as the pipeline runs it;
/// its UnitLeaderTraits table and its reads of unit leader objects.
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
    let report = copy_characters_growing(&memory, context(PART_MASKS[3]), job, Roles::Borrow(loan));
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

/// A traits job whose leaders' rows are carried under `leaders`, and the
/// role census it borrows.
fn trait_job(leaders: &'static LeaderWatches, world: &World) -> (CharactersJob, CharactersJob) {
    let mut job = CharactersJob::new_part(3);
    job.trait_carry = TraitCarry::with(leaders, leader_carry::TRAITS, Some(Check::Nothing));
    (job, census_tests::roster(world))
}

/// The typed walk's UnitLeaderTraits.
fn traits_walked(world: &World) -> TableView {
    census_tests::typed(world, 3).tables()[4].clone()
}

/// Leader `index`'s first assigned trait now names leader `other`'s.
fn swap_trait(world: &mut World, index: usize, other: usize) {
    let mut definition = [0; 8];
    assert!(world.copy(census_tests::TRAITS + other * 0x40, &mut definition));
    world
        .patches
        .push((census_tests::TRAITS + index * 0x40, definition.to_vec()));
}

/// Once the traits job held every leader's rows, an hour reads no unit
/// leader and publishes the typed walk's rows. A forwarded writer that left
/// the vector unchanged (the recalculation tail) makes only its leader be
/// read again; one that changed it publishes the new rows.
#[test]
fn unchanged_leaders_carry_their_traits_without_reading_them() {
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

/// A trait write no forwarded site observed keeps a carried leader's rows
/// stale until its slice reads it: that hour publishes the fresh rows and
/// poisons reuse, so the next hour reads every leader.
#[test]
fn the_slice_catches_a_missed_trait_write() {
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    LEADERS.enable(world.characters);
    let (mut job, owner) = trait_job(&LEADERS, &world);
    let loan = owner.role_census().unwrap();
    trait_hour(&mut job, &world, &loan);
    swap_trait(&mut world, 13, 9);
    let (stale, _) = trait_hour(&mut job, &world, &loan);
    assert_ne!(stale, traits_walked(&world));
    let id: u32 = 2_000_000 + 13;
    job.trait_carry.set_check(Check::Slice {
        hour: (id % 24) as i32,
    });
    let (caught, _) = trait_hour(&mut job, &world, &loan);
    assert_eq!(caught, traits_walked(&world));
    let counts = job.trait_carry.counts();
    assert_eq!(counts.mismatches, 1);
    assert!(LEADERS.poisoned());
    trait_hour(&mut job, &world, &loan);
    let counts = job.trait_carry.counts();
    assert_eq!((counts.reused, counts.read), (0, leaders(&world)));
}

/// A roster hour decoded into `frame`, which keeps its buffers between hours
/// as a pipeline frame does; its Characters and UnitLeaders tables.
fn roster_hour_into(
    job: &mut CharactersJob,
    world: &World,
    frame: &mut Captured,
) -> (TableView, TableView) {
    let report = copy_characters_growing(world, context(PART_MASKS[0]), job, Roles::Gather);
    assert!(report.raw_ok && !report.plan_failed);
    // The pipeline grows every frame between hours.
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

/// A frame whose buffers hold a quiet hour's Characters and UnitLeaders
/// tables publishes them again, without rewriting them, while the next
/// quiet hour's keys are theirs and every leader it reads afresh equals its
/// held row (a marker planted in the buffers survives); a changed leader
/// rewrites the UnitLeaders table from the walk.
#[test]
fn a_frame_publishes_its_held_tables_without_rewriting_them() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS, Check::Nothing);
    settle_roster(&mut job, &world);
    let mut frame = Captured::with_capacities(&capacities());
    for _ in 0..2 {
        let (characters, units) = roster_hour_into(&mut job, &world, &mut frame);
        assert_eq!(characters, walked(&world).tables()[0]);
        assert_eq!(units, walked(&world).tables()[3]);
    }
    const MARK: u32 = 0x5eed;
    frame.characters.rows.rows[0].country = MARK;
    frame.units.rows.rows[0].native.owner = MARK;
    // A notified leader whose row did not change is read and equals its
    // held row.
    drop(LEADERS.notify(World::unit(9).unwrap(), 0));
    roster_hour_into(&mut job, &world, &mut frame);
    assert_eq!(job.unit_carry.counts().read, 1);
    assert_eq!(
        frame.characters.rows.rows[0].country, MARK,
        "rows republished in place"
    );
    assert_eq!(
        frame.units.rows.rows[0].native.owner, MARK,
        "rows republished in place"
    );
    // A changed leader: the walk rewrites the table.
    world.patches.push((
        World::unit(5).unwrap() + cunit_leader::OWNER,
        7_u32.to_le_bytes().to_vec(),
    ));
    drop(LEADERS.notify(World::unit(5).unwrap(), 13));
    let (_, units) = roster_hour_into(&mut job, &world, &mut frame);
    assert_eq!(units, walked(&world).tables()[3]);
    // A fresh frame holds nothing and decodes the same tables.
    let (characters, units) = roster_hour_into(
        &mut job,
        &world,
        &mut Captured::with_capacities(&capacities()),
    );
    assert_eq!(characters, walked(&world).tables()[0]);
    assert_eq!(units, walked(&world).tables()[3]);
}

/// The traits job of a role hour decoded into `frame`, which keeps its
/// buffers between hours as a pipeline frame does.
fn trait_hour_into(
    job: &mut CharactersJob,
    world: &World,
    loan: &census::RoleCensus,
    frame: &mut Captured,
) -> TableView {
    let report = copy_characters_growing(world, context(PART_MASKS[3]), job, Roles::Borrow(loan));
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
    frame.tables()[4].clone()
}

/// A frame whose buffers hold a traits job's table publishes it again
/// without rewriting it while the table key is the carry's and every leader
/// read afresh projects its held rows (a planted marker survives, also after
/// an unchanged leader is read again); a changed vector rewrites the table.
#[test]
fn a_frame_publishes_its_held_traits_without_rewriting_them() {
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    LEADERS.enable(world.characters);
    let (mut job, owner) = trait_job(&LEADERS, &world);
    let loan = owner.role_census().unwrap();
    let mut frame = Captured::with_capacities(&capacities());
    for _ in 0..2 {
        assert_eq!(
            trait_hour_into(&mut job, &world, &loan, &mut frame),
            traits_walked(&world)
        );
    }
    const MARK: i32 = 0x5eed;
    frame.traits.rows.rows[0].token_id = MARK;
    drop(LEADERS.notify(World::unit(5).unwrap(), 0));
    trait_hour_into(&mut job, &world, &loan, &mut frame);
    assert_eq!(job.trait_carry.counts().read, 1);
    assert_eq!(
        frame.traits.rows.rows[0].token_id, MARK,
        "rows republished in place"
    );
    swap_trait(&mut world, 5, 9);
    drop(LEADERS.notify(World::unit(5).unwrap(), 0));
    assert_eq!(
        trait_hour_into(&mut job, &world, &loan, &mut frame),
        traits_walked(&world)
    );
}

#[test]
fn forced_full_refresh_publishes_current_training_without_a_stale_carry_key() {
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS, Check::Nothing);
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
    let progress = engine_native::read::<u64>(&world, leader + cunit_leader::TRAIT_EXPERIENCE_DATA)
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
        let report = copy_characters_growing(&world, ctx, &mut job, Roles::Gather);
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
    static WATCHES: Watches = Watches::new();
    static DOMAIN: Isolated = Isolated::new();
    static LEADERS: LeaderWatches = LeaderWatches::new();
    let mut world = world();
    WATCHES.enable(world.characters);
    DOMAIN.enable();
    LEADERS.enable(world.characters);
    let mut job = unit_job(&WATCHES, &DOMAIN, &LEADERS, Check::Nothing);
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
