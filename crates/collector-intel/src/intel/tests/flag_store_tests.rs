//! ScriptFlags rows carried between hours per flag manager
//! ([`super::super::FlagStore`], [`crate::flag_changes`]): quiet hours read no
//! manager and publish the same rows, a hooked write re-reads only its
//! manager and an invalidation re-reads every manager.
use super::*;
use crate::flag_changes::{Counts, Watches};
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

/// Add empty historical scopes without adding flag rows: watch demand must
/// follow scope count, not the usually much smaller live flag count.
fn with_empty_characters(count: usize) -> Vec<u8> {
    let mut bytes = flag_scopes();
    let list = 0x10000;
    let stride = 0x200;
    let characters = (list + (count + 2) * 8).next_multiple_of(stride);
    bytes.resize(characters + count * stride, 0);
    let historical = 0x5000 + ccharacter_manager::HISTORICAL_CHARACTERS;
    put(&mut bytes, historical, &(list as u64).to_le_bytes());
    for field in [8, 12] {
        put(
            &mut bytes,
            historical + field,
            &((count + 2) as u32).to_le_bytes(),
        );
    }
    for (index, character) in [0x6000_u64, 0x6200].into_iter().enumerate() {
        put(&mut bytes, list + index * 8, &character.to_le_bytes());
    }
    for index in 0..count {
        let character = characters + index * stride;
        put(
            &mut bytes,
            list + (index + 2) * 8,
            &(character as u64).to_le_bytes(),
        );
        for (field, vtable) in [
            (0, ccharacter::VTABLE_RVA),
            (ccharacter::FLAGS, cflag_manager::VTABLE_RVA),
        ] {
            put(
                &mut bytes,
                character + field,
                &((BASE + vtable) as u64).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            character + ccharacter::TYPE,
            &ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::ID,
            &((index + 100) as u32).to_le_bytes(),
        );
    }
    bytes
}

struct Hour {
    outcome: Outcome,
    rows: Vec<ScopedFlag>,
    counts: Counts,
    /// Guarded reads of any manager header or dense row.
    manager_reads: usize,
}

fn walk(bytes: &[u8], store: &mut FlagStore, watches: &Watches) -> Hour {
    walk_with(bytes, store, watches).0
}

/// [`walk`], also returning the guarded reads of any scope root, state,
/// character or manager, none of which a quiet hour makes.
fn walk_with(bytes: &[u8], store: &mut FlagStore, watches: &Watches) -> (Hour, usize) {
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
        Some(FlagReuse::begin(store, watches)),
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
    let first = walk(bytes, store, watches);
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
        let quiet = walk(&bytes, &mut store, &watches);
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
    let hour = walk(&bytes, &mut store, &watches);
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
    let quiet = walk(&bytes, &mut store, &watches);
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
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!((hour.counts.read, hour.counts.reused), (5, 0));
    assert_eq!(hour.rows, first.rows);
    // A forgotten store (a failed job) reads every manager too.
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&bytes, &mut store, &watches);
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
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!((hour.counts.read, hour.counts.reused), (1, 4));
    assert_eq!(
        (hour.rows[3].scope_kind, hour.rows[3].scope_id),
        (CHARACTER_SCOPE, 46)
    );
}

#[test]
fn an_unobserved_write_is_carried_stale() {
    // Guards the premise of the family: only hooked writers move a
    // certificate, so a missed writer would be carried until an
    // invalidation, which is why every writer site is hooked.
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    first_hour(&bytes, &mut store, &watches);
    set_value(&mut bytes, 0, 5);
    let stale = walk(&bytes, &mut store, &watches);
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
    let failed = walk(&broken, &mut store, &watches);
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
    // The failed walk's carried rows certify nothing afterwards.
    set_value(&mut bytes, 0, 5);
    let hour = walk(&bytes, &mut store, &watches);
    assert_eq!((hour.counts.read, hour.counts.reused), (5, 0));
    assert_eq!(hour.rows[0].flag.value, 5);
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
    let (first, reads) = walk_with(&bytes, &mut store, &watches);
    assert_ne!(reads, 0);
    assert!(!first.counts.quiet);
    // The held walk's rows are what it published.
    let held = store.witnessed().unwrap();
    watches.boundary(false);
    for _ in 0..3 {
        let (quiet, reads) = walk_with(&bytes, &mut store, &watches);
        assert_eq!(quiet.outcome, first.outcome);
        assert_eq!(quiet.rows, first.rows);
        assert!(quiet.counts.quiet);
        assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 5));
        assert_eq!(reads, 0);
        assert_eq!(store.witnessed(), Some(held));
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
    walk_with(&bytes, &mut store, &watches);
    let held = store.witnessed().unwrap();
    watches.boundary(false);
    let notification = watches.notify(MANAGERS[1], 0);
    set_value(&mut bytes, 1, 7);
    drop(notification);
    watches.boundary(false);
    let (written, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(written.counts.quiet);
    assert_eq!((written.counts.read, written.counts.reused), (1, 4));
    // Changed rows are new held rows.
    let renewed = store.witnessed().unwrap();
    assert_ne!(renewed, held);
    // Only the marked manager's header and row were read.
    assert_eq!(reads, written.manager_reads);
    assert_ne!(written.manager_reads, 0);
    assert_eq!(written.rows[1].flag.value, 7);
    assert_eq!(
        written.rows,
        walk(&bytes, &mut FlagStore::new(), &watches).rows
    );
    watches.boundary(false);
    let (quiet, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(quiet.counts.quiet);
    assert_eq!(reads, 0);
    assert_eq!(quiet.rows, written.rows);
    assert_eq!(store.witnessed(), Some(renewed));
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
    put(
        &mut bytes,
        character + ccharacter::FLAGS + cflag_manager::ROWS,
        &(row(3) as u64).to_le_bytes(),
    );
    for field in [cflag_manager::SLOT_COUNT, cflag_manager::ENTRY_COUNT] {
        put(
            &mut bytes,
            character + ccharacter::FLAGS + field,
            &1_i32.to_le_bytes(),
        );
    }
    // Unseen, the quiet hour still publishes the held rows...
    let (stale, _) = walk_with(&bytes, &mut store, &watches);
    assert!(stale.counts.quiet);
    watches.boundary(false);
    // ...and the roster write that appends it walks every scope again.
    watches.roster_write();
    watches.boundary(false);
    let (appended, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(!appended.counts.quiet);
    assert_ne!(reads, 0);
    // The country changed since the last full walk too: its old per-scope
    // certificate conservatively rereads it when the scope census resumes.
    assert_eq!((appended.counts.read, appended.counts.reused), (2, 4));
    assert_eq!(appended.outcome, Outcome::Rows { len: 5 });
    assert_eq!(
        appended.rows,
        walk(&bytes, &mut FlagStore::new(), &watches).rows
    );
    watches.boundary(false);
    // Removing that scope revokes the roster token too. No surviving
    // manager changed; only its scope membership disappears.
    put(
        &mut bytes,
        historical + engine_native::cpdx_array::COUNT,
        &2_u32.to_le_bytes(),
    );
    watches.roster_write();
    watches.boundary(false);
    let (removed, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(!removed.counts.quiet);
    assert_ne!(reads, 0);
    assert_eq!((removed.counts.read, removed.counts.reused), (0, 5));
    assert_eq!(removed.outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        removed.rows,
        walk(&bytes, &mut FlagStore::new(), &watches).rows
    );
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
/// character's embedded manager is carried by a quiet hour, which reads no
/// manager.
#[test]
fn a_quiet_hour_carries_an_unhooked_character_flag_write() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches);
    watches.boundary(false);
    set_value(&mut bytes, 3, 9);
    let (stale, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(stale.counts.quiet);
    assert_eq!(reads, 0);
    assert_eq!(stale.rows[3].flag.value, i16::MIN);
}

/// A quiet hour's publishing loop visits only the scopes it reads: none in
/// an unwritten hour; the written manager's scope after a write, including
/// one whose rows shrink. The held rows between them are copied whole, and
/// every hour's rows are a fresh walk's.
#[test]
fn a_quiet_hour_visits_only_the_scopes_it_reads() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches);
    watches.boundary(false);
    let (quiet, _) = walk_with(&bytes, &mut store, &watches);
    assert!(quiet.counts.quiet);
    assert!(FlagStore::visited_scopes().is_empty());
    watches.boundary(false);
    // A value written in place: the country scope alone.
    let notification = watches.notify(MANAGERS[1], 0);
    set_value(&mut bytes, 1, 7);
    drop(notification);
    watches.boundary(false);
    let (written, _) = walk_with(&bytes, &mut store, &watches);
    assert!(written.counts.quiet);
    // Walk order: the global scope, country 2, state 1, then characters.
    assert_eq!(FlagStore::visited_scopes(), [1]);
    assert_eq!(
        written.rows,
        walk(&bytes, &mut FlagStore::new(), &watches).rows
    );
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
    let (cleared, _) = walk_with(&bytes, &mut store, &watches);
    assert!(cleared.counts.quiet);
    assert_eq!(FlagStore::visited_scopes(), [1]);
    let fresh = walk(&bytes, &mut FlagStore::new(), &watches).rows;
    assert_eq!(cleared.rows, fresh);
    assert_eq!(cleared.rows.len(), 3);
    // The renewed rows, re-indexed, are carried whole the next hour.
    watches.boundary(false);
    let (next, _) = walk_with(&bytes, &mut store, &watches);
    assert!(next.counts.quiet);
    assert!(FlagStore::visited_scopes().is_empty());
    assert_eq!(next.rows, fresh);
}

/// MIO sizing can offer a table first. The existing ScriptFlags sizing
/// census must replace that offer with the combined demand before install;
/// waiting for the first capture leaves growth stuck until an invalidation.
#[test]
fn preinstall_flag_sizing_includes_empty_scopes_and_embedded_stores() {
    let bytes = with_empty_characters(5_000);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let scopes = 5_005;
    // One watch past a sizing boundary: omitting even one scope leaves the
    // smaller offer, while the embedded-only offer matches the undersizing.
    let embedded = 8_193 - scopes;
    crate::flag_changes::offer_embedded(embedded);
    assert_eq!(script_flag_demand(&memory, BASE, GAME), Some(4));
    crate::flag_changes::boundary(false);
    let status = serde_json::to_value(crate::flag_changes::status()).unwrap();
    let capacity = status["watches"]["capacity"].as_u64().unwrap() as usize;
    assert_eq!(capacity, ((scopes + embedded) * 4).next_power_of_two());
    assert_eq!(status["watches"]["installs"], 1);

    // The same prepared demand certifies every scope, even when almost all
    // managers are empty. The next hour walks and reads none of them.
    let watches = Watches::enabled(scopes + embedded);
    let mut store = FlagStore::new();
    let (first, _) = walk_with(&bytes, &mut store, &watches);
    assert_eq!(first.outcome, Outcome::Rows { len: 4 });
    assert_eq!((first.counts.read, first.counts.reused), (scopes as u64, 0));
    watches.boundary(false);
    let (quiet, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(quiet.counts.quiet);
    assert_eq!(quiet.outcome, first.outcome);
    assert_eq!(quiet.rows, first.rows);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, scopes as u64));
    assert_eq!(reads, 0);
    assert!(FlagStore::visited_scopes().is_empty());
}

#[test]
fn refused_flag_sizing_keeps_the_prior_watch_demand() {
    let mut bytes = with_empty_characters(5_000);
    // The last manager's invalid header refuses the entire sizing census,
    // not a successful partial scope count.
    let last = bytes.len() - 0x200;
    put(&mut bytes, last + ccharacter::FLAGS, &0_u64.to_le_bytes());
    crate::flag_changes::offer_embedded(1_024);
    assert_eq!(
        script_flag_demand(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            GAME
        ),
        None
    );
    crate::flag_changes::boundary(false);
    let status = serde_json::to_value(crate::flag_changes::status()).unwrap();
    assert_eq!(status["watches"]["capacity"], 4_096);
}

#[test]
fn refused_flag_watches_read_fresh_instead_of_certifying_a_quiet_hour() {
    use crate::flag_changes::Changes;
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(1);
    // Fill the real bounded watch table with unrelated receivers.
    for receiver in 1..=16_384 {
        let _ = watches.sequence(receiver);
    }
    assert!(
        MANAGERS
            .iter()
            .all(|&manager| watches.sequence(manager).is_none())
    );
    let mut store = FlagStore::new();
    let (first, _) = walk_with(&bytes, &mut store, &watches);
    assert_eq!(first.outcome, Outcome::Rows { len: 4 });
    watches.boundary(false);
    set_value(&mut bytes, 1, 7);
    let (fresh, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(!fresh.counts.quiet);
    assert_eq!((fresh.counts.read, fresh.counts.reused), (5, 0));
    assert_ne!(reads, 0);
    assert_eq!(fresh.rows[1].flag.value, 7);
}

#[test]
fn a_marked_flag_read_refusal_never_publishes_the_held_table() {
    let mut bytes = flag_scopes();
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    walk_with(&bytes, &mut store, &watches);
    watches.boundary(false);
    let notification = watches.notify(MANAGERS[1], 0);
    put(&mut bytes, MANAGERS[1], &0_u64.to_le_bytes());
    drop(notification);
    watches.boundary(false);
    let (refused, _) = walk_with(&bytes, &mut store, &watches);
    let fresh = walk(&bytes, &mut FlagStore::new(), &watches);
    assert_eq!(
        refused.outcome,
        Outcome::Unavailable {
            reason: "flag manager vtable mismatch"
        }
    );
    assert_eq!(refused.outcome, fresh.outcome);
    assert_eq!(refused.rows, fresh.rows);
    assert!(refused.rows.is_empty());
    assert!(!refused.counts.quiet);
    // No failed acquisition remains eligible for quiet carry.
    put(
        &mut bytes,
        MANAGERS[1],
        &((BASE + cflag_manager::VTABLE_RVA) as u64).to_le_bytes(),
    );
    watches.boundary(false);
    let (recovered, _) = walk_with(&bytes, &mut store, &watches);
    assert_eq!((recovered.counts.read, recovered.counts.reused), (5, 0));
    assert_eq!(recovered.outcome, Outcome::Rows { len: 4 });
}

#[test]
fn a_daily_expiry_mark_refreshes_the_countdown_and_removal_only() {
    let mut bytes = flag_scopes();
    put(
        &mut bytes,
        row(1) + cscript_flag::REMAINING_DAYS,
        &2_i16.to_le_bytes(),
    );
    let watches = Watches::enabled(16);
    let mut store = FlagStore::new();
    let (first, _) = walk_with(&bytes, &mut store, &watches);
    watches.boundary(false);
    // Advancing an ordinary hour changes neither the stored set hour nor
    // the native remaining-days field, even for a timed flag.
    let (quiet, reads) = walk_with(&bytes, &mut store, &watches);
    assert!(quiet.counts.quiet);
    assert_eq!(quiet.rows, first.rows);
    assert_eq!(reads, 0);
    for remaining in [1_i16, 0] {
        let notification = watches.notify(MANAGERS[1], 3);
        put(
            &mut bytes,
            row(1) + cscript_flag::REMAINING_DAYS,
            &remaining.to_le_bytes(),
        );
        if remaining == 0 {
            put(
                &mut bytes,
                MANAGERS[1] + cflag_manager::ENTRY_COUNT,
                &0_i32.to_le_bytes(),
            );
        }
        drop(notification);
        watches.boundary(false);
        let (expired, _) = walk_with(&bytes, &mut store, &watches);
        assert!(expired.counts.quiet);
        assert_eq!((expired.counts.read, expired.counts.reused), (1, 4));
        assert_eq!(FlagStore::visited_scopes(), [1]);
        assert_eq!(
            expired.outcome,
            Outcome::Rows {
                len: if remaining == 0 { 3 } else { 4 }
            }
        );
        if remaining != 0 {
            assert_eq!(expired.rows[1].flag.remaining_days, remaining);
        }
        assert_eq!(
            expired.rows,
            walk(&bytes, &mut FlagStore::new(), &watches).rows
        );
    }
}
