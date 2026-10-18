//! ScriptFlags rows carried between hours per flag manager
//! ([`super::super::FlagStore`], [`crate::flag_changes`]): quiet hours read no
//! manager and publish the same rows, a hooked write re-reads only its
//! manager, an invalidation re-reads every manager, and verify mode catches
//! a write no hook saw and poisons reuse.
use super::*;
use crate::{
    flag_changes::{Counts, Watches},
    sampled_verify::{Check, ROTATION},
};
use std::cell::RefCell;

/// Every guarded call's first address, over the [`flag_scopes`] bytes.
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

/// The five scope managers of [`flag_scopes`]: global, country 2, state 1
/// and characters 44 and 45 (embedded, the last one empty).
const MANAGERS: [usize; 5] = [
    0xa000,
    0xa100,
    0xa200,
    0x6000 + ccharacter::FLAGS,
    0x6200 + ccharacter::FLAGS,
];
/// The dense row each of the first four managers holds.
fn row(scope: usize) -> usize {
    0xb000 + scope * cscript_flag::STRIDE
}

struct Hour {
    outcome: Outcome,
    rows: Vec<ScopedFlag>,
    counts: Counts,
    /// Guarded reads of any manager header or dense row.
    manager_reads: usize,
}

fn walk(bytes: &[u8], store: &mut FlagStore, watches: &Watches, check: Check) -> Hour {
    walk_with(bytes, store, watches, check, crate::roster_changes::VERIFY).0
}

/// [`walk`], publishing a quiet hour's held rows unless `quiet_verify`; also
/// returns the guarded reads of any scope root, state, character or manager,
/// none of which a quiet hour makes.
fn walk_with(
    bytes: &[u8],
    store: &mut FlagStore,
    watches: &Watches,
    check: Check,
    quiet_verify: bool,
) -> (Hour, usize) {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let mut rows = Vec::new();
    let mut scratch = Vec::new();
    let outcome = walk_flags_growing(
        &memory,
        BASE,
        GAME,
        &mut rows,
        &mut scratch,
        Some(FlagReuse::begin(store, watches, check).verifying_quiet(quiet_verify)),
    );
    rows.truncate(outcome.len());
    let calls = memory.calls.into_inner();
    let scope_reads = calls
        .iter()
        .filter(|&&address| {
            [
                cgame_state::GLOBAL_FLAGS,
                cgame_state::STATES,
                cgame_state::CHARACTER_MANAGER,
            ]
            .iter()
            .any(|&offset| address == GAME + offset)
                || (0x2200..0x2300).contains(&address)
                || (0x5000..0xc000).contains(&address)
        })
        .count();
    let manager_reads = calls
        .into_iter()
        .filter(|&address| {
            MANAGERS[..3]
                .iter()
                .any(|&manager| (manager..manager + cflag_manager::HEADER).contains(&address))
                || (0xb000..0xb000 + 4 * cscript_flag::STRIDE).contains(&address)
        })
        .count();
    (
        Hour {
            outcome,
            rows,
            counts: store.counts(),
            manager_reads,
        },
        scope_reads,
    )
}

fn set_value(bytes: &mut [u8], scope: usize, value: i16) {
    put(
        bytes,
        row(scope) + cscript_flag::VALUE,
        &value.to_le_bytes(),
    );
}

/// The first hour reads and holds every manager; this is what the old
/// behaviour does every hour.
fn first_hour(bytes: &[u8], store: &mut FlagStore, watches: &Watches) -> Hour {
    let first = walk(bytes, store, watches, Check::Nothing);
    assert_eq!(first.outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        (
            first.counts.read,
            first.counts.reused,
            first.counts.read_rows
        ),
        (5, 0, 4)
    );
    assert_ne!(first.manager_reads, 0);
    watches.boundary(false);
    first
}

#[test]
fn quiet_hours_read_no_manager_and_publish_identical_rows() {
    let bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    for _ in 0..3 {
        let quiet = walk(&bytes, &mut store, &watches, Check::Nothing);
        assert_eq!(quiet.outcome, first.outcome);
        assert_eq!(quiet.rows, first.rows);
        assert_eq!(
            (
                quiet.counts.read,
                quiet.counts.reused,
                quiet.counts.reused_rows
            ),
            (0, 5, 4)
        );
        // Neither a header nor a row of a non-character manager is read; a
        // character's header still arrives with its identity.
        assert_eq!(quiet.manager_reads, 0);
        watches.boundary(false);
    }
}

#[test]
fn a_hooked_write_rereads_only_its_manager() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    // set_country_flag on country 2: the detour notifies around the write.
    let notification = watches.notify(MANAGERS[1], 0);
    set_value(&mut bytes, 1, 7);
    drop(notification);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!(hour.outcome, Outcome::Rows { len: 4 });
    assert_eq!((hour.counts.read, hour.counts.reused), (1, 4));
    assert_eq!(hour.rows[1].scope_kind, COUNTRY_SCOPE);
    assert_eq!(hour.rows[1].flag.value, 7);
    assert!(
        hour.rows
            .iter()
            .enumerate()
            .all(|(index, row)| index == 1 || row.flag.value == i16::MIN)
    );
    // The re-read manager is carried again from the next quiet hour.
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 5));
    assert_eq!(quiet.rows, hour.rows);
}

#[test]
fn an_invalidating_hour_rereads_every_manager() {
    let bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    // Load, gap or recovery.
    watches.boundary(true);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((hour.counts.read, hour.counts.reused), (5, 0));
    assert_eq!(hour.rows, first.rows);
    // A forgotten store (a failed job) reads every manager too.
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((forgotten.counts.read, forgotten.counts.reused), (5, 0));
}

#[test]
fn a_changed_scope_identity_is_read_afresh() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    // Character 44 becomes 46 at the same address and manager. Only
    // character identity repair rewrites a rostered character's id
    // (`RegisterIdentity`), and the roster domain observes it, so the hour
    // walks every scope and re-reads only the scope whose identity changed.
    put(&mut bytes, 0x6000 + ccharacter::ID, &46_u32.to_le_bytes());
    watches.roster_write();
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((hour.counts.read, hour.counts.reused), (1, 4));
    assert_eq!(
        (hour.rows[3].scope_kind, hour.rows[3].scope_id),
        (CHARACTER_SCOPE, 46)
    );
}

#[test]
fn verify_catches_an_unobserved_write_and_poisons_reuse() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    // Verify of an unchanged hour reads every carryable manager and agrees.
    let agreed = walk(&bytes, &mut store, &watches, Check::Every);
    assert_eq!(
        (
            agreed.counts.read,
            agreed.counts.verified,
            agreed.counts.mismatches
        ),
        (5, 5, 0)
    );
    assert!(!watches.poisoned());
    watches.boundary(false);
    // A write no hook observed.
    set_value(&mut bytes, 2, 9);
    let caught = walk(&bytes, &mut store, &watches, Check::Every);
    // The state's mismatch poisons at once: the characters after it are no
    // longer carryable, so they are read but not verified.
    assert_eq!(
        (
            caught.counts.read,
            caught.counts.verified,
            caught.counts.mismatches
        ),
        (5, 3, 1)
    );
    assert!(watches.poisoned());
    // The fresh rows are published.
    assert_eq!(caught.rows[2].scope_kind, STATE_SCOPE);
    assert_eq!(caught.rows[2].flag.value, 9);
    // Without verify, a poisoned domain carries nothing.
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((after.counts.read, after.counts.reused), (5, 0));
    assert_eq!(after.rows[2].flag.value, 9);
}

#[test]
fn an_unobserved_write_without_verify_is_carried_stale() {
    // Guards the premise of the family: only hooked writers move a
    // certificate, so a missed writer would be carried until verify or an
    // invalidation, which is why every writer site is hooked.
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    set_value(&mut bytes, 0, 5);
    let stale = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!(stale.counts.reused, 5);
    assert_eq!(stale.rows[0].flag.value, i16::MIN);
}

#[test]
fn a_failed_walk_holds_nothing() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    // A state's flag root is written only by state construction
    // and destroy state, which build and destroy its manager through
    // the hooked construct flags and destroy flags: the state's manager is
    // destroyed (`destroy_flags`) and its root no longer reads.
    let mut broken = bytes.clone();
    let notification = watches.notify(MANAGERS[2], 9);
    put(&mut broken, 0x8000 + cstate::FLAGS, &0_u64.to_le_bytes());
    drop(notification);
    watches.boundary(false);
    let failed = walk(&broken, &mut store, &watches, Check::Nothing);
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
    // The failed walk's carried rows certify nothing afterwards.
    set_value(&mut bytes, 0, 5);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((hour.counts.read, hour.counts.reused), (5, 0));
    assert_eq!(hour.rows[0].flag.value, 5);
}

#[test]
fn an_hourly_slice_verifies_only_its_scopes_and_carries_the_rest() {
    let bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    // Global 0, country 2, state 1 and characters 44 and 45, by kind and id.
    let keys = [
        flag_store::key(GLOBAL_SCOPE, 0),
        flag_store::key(COUNTRY_SCOPE, 2),
        flag_store::key(STATE_SCOPE, 1),
        flag_store::key(CHARACTER_SCOPE, 44),
        flag_store::key(CHARACTER_SCOPE, 45),
    ];
    let mut verified = 0;
    for hour in 0..ROTATION as i32 {
        let check = Check::Slice { hour };
        let sampled = walk(&bytes, &mut store, &watches, check);
        let due = keys.iter().filter(|&&key| check.covers(key)).count() as u64;
        assert_eq!(
            (sampled.counts.verified, sampled.counts.reused),
            (due, 5 - due)
        );
        assert_eq!(sampled.counts.mismatches, 0);
        verified += due;
        watches.boundary(false);
    }
    // One rotation compares every carried scope exactly once.
    assert_eq!(verified, 5);
}

#[test]
fn every_site_names_its_manager() {
    use engine_mutations::{
        script_flag_mutations::MUTATIONS, script_flag_mutations::Receiver,
        script_flag_mutations::manager,
    };
    for (site, receiver) in MUTATIONS {
        let expected = match site.name {
            "assign_flags" => Receiver::Rows,
            _ => Receiver::Manager,
        };
        assert_eq!(receiver, expected, "{}", site.name);
    }
    assert_eq!(manager(Receiver::Manager, 0xa100), 0xa100);
    // Assign flags receives the manager's row array.
    assert_eq!(manager(Receiver::Rows, 0xa108), 0xa100);
    let watches = Watches::enabled(16);
    let certified = crate::flag_changes::Changes::sequence(&watches, 0xa100).unwrap();
    drop(watches.notify(manager(Receiver::Rows, 0xa108), 6));
    assert_ne!(
        crate::flag_changes::Changes::sequence(&watches, 0xa100),
        Some(certified)
    );
}

/// A quiet hour (no flag or roster writer since the latest complete walk,
/// the same countries) walks no scope at all and publishes that walk's rows;
/// the old walk read every root, state, character and manager header each
/// hour.
#[test]
fn a_quiet_hour_walks_no_scope_and_publishes_identical_rows() {
    let bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    let (first, reads) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert_ne!(reads, 0);
    assert!(!first.counts.quiet);
    watches.boundary(false);
    for _ in 0..3 {
        let (quiet, reads) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
        assert_eq!(quiet.outcome, first.outcome);
        assert_eq!(quiet.rows, first.rows);
        assert!(quiet.counts.quiet);
        assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 5));
        assert_eq!(reads, 0);
        watches.boundary(false);
    }
}

/// A flag write marks its manager: the hour is still quiet, walks no scope
/// and reads only that manager afresh, and the next unwritten hour publishes
/// the new rows without reading it. A roster write moves a token: that hour
/// walks every scope.
#[test]
fn a_flag_write_rereads_its_manager_and_a_roster_write_walks_every_scope() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    watches.boundary(false);
    let notification = watches.notify(MANAGERS[1], 0);
    set_value(&mut bytes, 1, 7);
    drop(notification);
    watches.boundary(false);
    let (written, reads) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(written.counts.quiet);
    assert_eq!((written.counts.read, written.counts.reused), (1, 4));
    // Only the marked manager's header and row were read.
    assert_eq!(reads, written.manager_reads);
    assert_ne!(written.manager_reads, 0);
    assert_eq!(written.rows[1].flag.value, 7);
    assert_eq!(
        written.rows,
        walk(&bytes, &mut FlagStore::new(), &watches, Check::Nothing).rows
    );
    watches.boundary(false);
    let (quiet, reads) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(quiet.counts.quiet);
    assert_eq!(reads, 0);
    assert_eq!(quiet.rows, written.rows);
    watches.boundary(false);
    // A third historical character, 47, owning one flag, is appended.
    let character = 0x6400;
    put(&mut bytes, 0x5110, &(character as u64).to_le_bytes());
    let historical = 0x5000 + ccharacter_manager::HISTORICAL_CHARACTERS;
    put(&mut bytes, historical + 8, &3_u32.to_le_bytes());
    put(&mut bytes, historical + 12, &3_u32.to_le_bytes());
    put(
        &mut bytes,
        character,
        &((BASE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        character + ccharacter::TYPE,
        &ccharacter::REFERENCE_TYPE.to_le_bytes(),
    );
    put(
        &mut bytes,
        character + ccharacter::ID,
        &47_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        character + ccharacter::FLAGS,
        &((BASE + cflag_manager::VTABLE_RVA) as u64).to_le_bytes(),
    );
    // Unseen, the quiet hour still publishes the held rows...
    let (stale, _) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(stale.counts.quiet);
    watches.boundary(false);
    // ...and the roster write that appends it walks every scope again.
    watches.roster_write();
    watches.boundary(false);
    let (appended, reads) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(!appended.counts.quiet);
    assert_ne!(reads, 0);
    assert_eq!(appended.counts.read + appended.counts.reused, 6);
}

/// A quiet hour's slice reads its managers afresh: a write no hook saw
/// poisons the flag and roster domains, and that hour walks and publishes
/// every scope afresh.
#[test]
fn a_quiet_hour_slice_catches_a_missed_write_and_poisons_both_domains() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    watches.boundary(false);
    set_value(&mut bytes, 2, 9);
    let hour = (0..ROTATION as i32)
        .find(|&hour| Check::Slice { hour }.covers(flag_store::key(STATE_SCOPE, 1)))
        .unwrap();
    let (caught, reads) = walk_with(&bytes, &mut store, &watches, Check::Slice { hour }, false);
    assert!(!caught.counts.quiet);
    assert_ne!(reads, 0);
    assert_eq!(caught.counts.mismatches, 1);
    assert_eq!(caught.rows[2].flag.value, 9);
    assert!(watches.poisoned());
    assert!(watches.roster_poisoned());
}

/// While the roster family burns in, a hour whose key matched walks every
/// scope and compares the walk with the held rows: a scope change no hook saw
/// (a character's id) poisons both domains.
#[test]
fn verify_mode_compares_each_quiet_hour_with_a_fresh_walk() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches, Check::Nothing, true);
    watches.boundary(false);
    let (agreed, reads) = walk_with(&bytes, &mut store, &watches, Check::Nothing, true);
    assert!(!agreed.counts.quiet);
    assert_ne!(reads, 0);
    assert_eq!(agreed.counts.mismatches, 0);
    assert!(!watches.roster_poisoned());
    watches.boundary(false);
    put(&mut bytes, 0x6000 + ccharacter::ID, &46_u32.to_le_bytes());
    let (caught, _) = walk_with(&bytes, &mut store, &watches, Check::Nothing, true);
    assert_eq!(caught.rows[3].scope_id, 46);
    assert_eq!(caught.counts.mismatches, 1);
    assert!(watches.roster_poisoned());
}

/// The country census comparison: the same countries in walk order, and
/// nothing else.
#[test]
fn a_quiet_hour_needs_the_held_countries_in_walk_order() {
    let bytes = flag_scopes();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(same_countries(&memory, BASE, GAME, &[(0x4000, 2)]));
    assert!(!same_countries(&memory, BASE, GAME, &[(0x4000, 3)]));
    assert!(!same_countries(&memory, BASE, GAME, &[(0x4100, 2)]));
    assert!(!same_countries(&memory, BASE, GAME, &[]));
    assert!(!same_countries(
        &memory,
        BASE,
        GAME,
        &[(0x4000, 2), (0x4000, 2)]
    ));
}

/// The family's premise for a character scope: an unhooked write to a
/// character's embedded manager is carried by a quiet hour until the hour's
/// sampled slice reads that manager afresh, which publishes the fresh rows
/// and poisons the flag and roster domains.
#[test]
fn a_quiet_hour_slice_catches_an_unhooked_character_flag_write() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    watches.boundary(false);
    set_value(&mut bytes, 3, 9);
    let key = flag_store::key(CHARACTER_SCOPE, 44);
    // An hour whose slice holds none of the five scopes reads no manager.
    let keys = [
        flag_store::key(GLOBAL_SCOPE, 0),
        flag_store::key(COUNTRY_SCOPE, 2),
        flag_store::key(STATE_SCOPE, 1),
        key,
        flag_store::key(CHARACTER_SCOPE, 45),
    ];
    let outside = (0..ROTATION as i32)
        .find(|&hour| !keys.iter().any(|&key| Check::Slice { hour }.covers(key)))
        .unwrap();
    let (stale, reads) = walk_with(
        &bytes,
        &mut store,
        &watches,
        Check::Slice { hour: outside },
        false,
    );
    assert!(stale.counts.quiet);
    assert_eq!(reads, 0);
    assert_eq!(stale.rows[3].flag.value, i16::MIN);
    watches.boundary(false);
    let inside = (0..ROTATION as i32)
        .find(|&hour| Check::Slice { hour }.covers(key))
        .unwrap();
    let (caught, _) = walk_with(
        &bytes,
        &mut store,
        &watches,
        Check::Slice { hour: inside },
        false,
    );
    assert!(!caught.counts.quiet);
    assert_eq!(caught.counts.mismatches, 1);
    assert_eq!(caught.rows[3].scope_id, 44);
    assert_eq!(caught.rows[3].flag.value, 9);
    assert!(watches.poisoned());
    assert!(watches.roster_poisoned());
}

/// A quiet hour's publishing loop visits only the scopes it reads: none in
/// an unwritten, unsampled hour; the written manager's scope after a write,
/// including one whose rows shrink; the sampled scope in a sampled hour.
/// The held rows between them are copied whole, and every hour's rows are a
/// fresh walk's.
#[test]
fn a_quiet_hour_visits_only_the_scopes_it_reads() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    watches.boundary(false);
    let (quiet, _) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(quiet.counts.quiet);
    assert!(FlagStore::visited_scopes().is_empty());
    watches.boundary(false);
    // A value written in place: the country scope alone.
    let notification = watches.notify(MANAGERS[1], 0);
    set_value(&mut bytes, 1, 7);
    drop(notification);
    watches.boundary(false);
    let (written, _) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(written.counts.quiet);
    // Walk order: the global scope, country 2, state 1, then characters.
    assert_eq!(FlagStore::visited_scopes(), [1]);
    assert_eq!(
        written.rows,
        walk(&bytes, &mut FlagStore::new(), &watches, Check::Nothing).rows
    );
    watches.boundary(false);
    // A sampled hour reads and compares the scopes its slice covers, the
    // country's among them.
    let keys = [
        flag_store::key(GLOBAL_SCOPE, 0),
        flag_store::key(COUNTRY_SCOPE, 2),
        flag_store::key(STATE_SCOPE, 1),
        flag_store::key(CHARACTER_SCOPE, 44),
        flag_store::key(CHARACTER_SCOPE, 45),
    ];
    let sampled = Check::BurnedIn {
        hour: (keys[1] % crate::sampled_verify::BURNED_IN_ROTATION) as i32,
    };
    let due: Vec<u32> = (0..keys.len() as u32)
        .filter(|&scope| sampled.covers(keys[scope as usize]))
        .collect();
    assert!(due.contains(&1));
    let (compared, _) = walk_with(&bytes, &mut store, &watches, sampled, false);
    assert!(compared.counts.quiet);
    assert_eq!(compared.counts.verified, due.len() as u64);
    assert_eq!(FlagStore::visited_scopes(), due);
    assert_eq!(compared.rows, written.rows);
    watches.boundary(false);
    // The country's flag is cleared: its scope's rows shrink to none, and
    // every later scope's held rows move up.
    let notification = watches.notify(MANAGERS[1], 0);
    // Its manager now holds no rows, as the empty character manager does.
    for field in [cflag_manager::SLOT_COUNT, cflag_manager::ENTRY_COUNT] {
        put(&mut bytes, MANAGERS[1] + field, &0_i32.to_le_bytes());
    }
    put(
        &mut bytes,
        MANAGERS[1] + cflag_manager::ROWS,
        &0_u64.to_le_bytes(),
    );
    drop(notification);
    watches.boundary(false);
    let (cleared, _) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(cleared.counts.quiet);
    assert_eq!(FlagStore::visited_scopes(), [1]);
    let fresh = walk(&bytes, &mut FlagStore::new(), &watches, Check::Nothing).rows;
    assert_eq!(cleared.rows, fresh);
    assert_eq!(cleared.rows.len(), 3);
    // The renewed rows, re-indexed, are carried whole the next hour.
    watches.boundary(false);
    let (next, _) = walk_with(&bytes, &mut store, &watches, Check::Nothing, false);
    assert!(next.counts.quiet);
    assert!(FlagStore::visited_scopes().is_empty());
    assert_eq!(next.rows, fresh);
}
