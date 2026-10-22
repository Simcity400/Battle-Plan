//! ScriptVariables rows carried between hours ([`super::super::VariableStore`],
//! [`crate::variable_changes`]): a quiet hour walks no scope and reads only
//! the stores a hooked writer marked and its slice, publishing exactly a
//! fresh walk's rows; a destroyed or created store walks every scope; an
//! invalidation re-reads every store; and the slice and verify mode catch a
//! write no hook saw and poison reuse.
use super::*;
use crate::{
    sampled_verify::{Check, ROTATION},
    variable_changes::{Counts, Watches},
};
use engine_scripts::variable_hash_node as node;
use std::cell::RefCell;

/// Every guarded call's first address, over the [`variable_scopes`] bytes.
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

/// Buckets (mask 7) plus the probe tail of each fixture store.
const SLOTS: usize = 8 + 5;
/// The five stores: global, country 2, state 1, and characters 44 and 45
/// (embedded).
const STORES: [usize; 5] = [
    0xc000,
    0xc100,
    0xc200,
    0x6000 + ccharacter::VARIABLES,
    0x6200 + ccharacter::VARIABLES,
];
/// Each store's node storage.
fn storage(store: usize) -> usize {
    0xd000 + store * 0x400
}
/// The key of each store's one variable; the global store's key lives on
/// the heap.
const KEYS: [&[u8]; 5] = [
    b"global_variable_with_a_long_name",
    b"country_var",
    b"state_var",
    b"char_var",
    b"other_var",
];
/// A long key's heap bytes.
const HEAP_KEY: usize = 0xf800;

fn set_value(bytes: &mut [u8], store: usize, value: i64) {
    put(bytes, storage(store) + node::VALUE, &value.to_le_bytes());
}

fn set_seed(bytes: &mut [u8], store: usize, seed: u32) {
    put(bytes, STORES[store] + cvariables::SEED, &seed.to_le_bytes());
}

/// Inserts `key` (inline) in slot 1 of `store`'s table, its home bucket.
fn add_variable(bytes: &mut [u8], store: usize, key: &[u8], value: i64) {
    let slot = storage(store) + node::STRIDE;
    put(bytes, slot, &1_u32.to_le_bytes());
    bytes[slot + node::DISTANCE] = 1;
    bytes[slot + node::KEY..slot + node::KEY + key.len()].copy_from_slice(key);
    put(bytes, slot + 0x18, &(key.len() as u64).to_le_bytes());
    put(bytes, slot + 0x20, &15_u64.to_le_bytes());
    put(bytes, slot + node::VALUE, &value.to_le_bytes());
    put(
        bytes,
        STORES[store] + cvariables::ENTRY_COUNT,
        &2_u32.to_le_bytes(),
    );
}

/// Erases slot 1 of `store`'s table.
fn erase_variable(bytes: &mut [u8], store: usize) {
    let slot = storage(store) + node::STRIDE;
    bytes[slot..slot + node::STRIDE].fill(0);
    put(
        bytes,
        STORES[store] + cvariables::ENTRY_COUNT,
        &1_u32.to_le_bytes(),
    );
}

/// An empty store at `container`.
fn empty_store(bytes: &mut [u8], container: usize, seed: u32) {
    put(
        bytes,
        container,
        &((BASE + cvariables::VTABLE_RVA) as u64).to_le_bytes(),
    );
    put(bytes, container + cvariables::SEED, &seed.to_le_bytes());
    put(
        bytes,
        container + cvariables::STORAGE,
        &((BASE + cvariables::EMPTY_STORAGE_RVA) as u64).to_le_bytes(),
    );
}

/// [`flag_scopes`]' countries, states and characters, each with a variable
/// store holding one variable in slot 0.
fn variable_scopes() -> Vec<u8> {
    let mut bytes = flag_scopes();
    bytes.resize(0x10000, 0);
    for (store, (root, offset)) in [
        (Some(GAME), cgame_state::GLOBAL_VARIABLES),
        (Some(0x4000), ccountry::VARIABLES),
        (Some(0x8000), cstate::VARIABLES),
        (None, STORES[3]),
        (None, STORES[4]),
    ]
    .into_iter()
    .enumerate()
    {
        let container = match root {
            Some(root) => {
                put(
                    &mut bytes,
                    root + offset,
                    &(STORES[store] as u64).to_le_bytes(),
                );
                STORES[store]
            }
            None => offset,
        };
        put(
            &mut bytes,
            container,
            &((BASE + cvariables::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            container + cvariables::SEED,
            &(1000 + store as u32).to_le_bytes(),
        );
        let nodes = storage(store);
        put(
            &mut bytes,
            container + cvariables::STORAGE,
            &(nodes as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            container + cvariables::ENTRY_COUNT,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            container + cvariables::BUCKET_MASK,
            &7_u32.to_le_bytes(),
        );
        bytes[container + cvariables::PROBE_TAIL] = 5;
        bytes[nodes + SLOTS * node::STRIDE + node::DISTANCE] = u8::MAX;
        // Hash 0: home slot 0 at probe distance 1.
        bytes[nodes + node::DISTANCE] = 1;
        let key = KEYS[store];
        put(&mut bytes, nodes + 0x18, &(key.len() as u64).to_le_bytes());
        if key.len() <= 15 {
            bytes[nodes + node::KEY..nodes + node::KEY + key.len()].copy_from_slice(key);
            put(&mut bytes, nodes + 0x20, &15_u64.to_le_bytes());
        } else {
            put(
                &mut bytes,
                nodes + node::KEY,
                &(HEAP_KEY as u64).to_le_bytes(),
            );
            bytes[HEAP_KEY..HEAP_KEY + key.len()].copy_from_slice(key);
            put(&mut bytes, nodes + 0x20, &(key.len() as u64).to_le_bytes());
        }
        put(
            &mut bytes,
            nodes + node::VALUE,
            &(100_000 * (store as i64 + 1)).to_le_bytes(),
        );
    }
    bytes
}

/// One capture's published walk.
type Published = (
    cvariables::ScopeWalk,
    Vec<cvariables::ScopeRow>,
    Vec<cvariables::Row>,
    Vec<u8>,
);

struct Hour {
    walk: cvariables::ScopeWalk,
    scopes: Vec<cvariables::ScopeRow>,
    entries: Vec<cvariables::Row>,
    names: Vec<u8>,
    counts: Counts,
    /// Guarded reads of any store's nodes or heap key bytes.
    node_reads: usize,
    /// The fixture stores whose vtable, header or seed the hour read.
    stores_read: Vec<usize>,
    /// Guarded reads of the roster or a character.
    character_reads: usize,
}

/// One scope's identity, seed, and keys with their values.
type ScopeVariables = (cvariables::Scope, Option<u32>, Vec<(Vec<u8>, i64)>);

impl Hour {
    /// Each scope's seed, keys and values, independent of arena positions.
    fn rows(&self) -> Vec<ScopeVariables> {
        self.scopes
            .iter()
            .map(|scope| {
                let entries = &self.entries[scope.entries_start..][..scope.entries_len];
                (
                    scope.scope,
                    scope.seed,
                    entries
                        .iter()
                        .map(|entry| {
                            (
                                self.names[entry.name_start..][..entry.name_len].to_vec(),
                                entry.value_raw,
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn published(&self) -> Published {
        (
            self.walk,
            self.scopes.clone(),
            self.entries.clone(),
            self.names.clone(),
        )
    }
}

/// A walk without any held rows over the same bytes: what every hour must
/// publish, position for position.
fn fresh(bytes: &[u8]) -> Published {
    let memory = SliceMemory { base: 0, bytes };
    let (mut scopes, mut entries, mut names) = (Vec::new(), Vec::new(), Vec::new());
    let walk =
        cvariables::walk_scopes_growing(&memory, BASE, GAME, &mut scopes, &mut entries, &mut names);
    scopes.truncate(walk.outcome.len());
    entries.truncate(walk.entries_len);
    names.truncate(walk.bytes_len);
    (walk, scopes, entries, names)
}

/// One hour's capture as the IntelResidual job makes it: a quiet hour, or
/// the walk.
fn walk(bytes: &[u8], store: &mut VariableStore, watches: &Watches, check: Check) -> Hour {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let (mut scopes, mut entries, mut names) = (Vec::new(), Vec::new(), Vec::new());
    let mut reuse = VariableReuse::begin(store, watches, check);
    let walk = match reuse.quiet_hour(&memory, BASE, GAME, &mut scopes, &mut entries, &mut names) {
        Some(walk) => walk,
        None => cvariables::walk_scopes_carried(
            &memory,
            BASE,
            GAME,
            engine_native::walk::Buffer::Growing(&mut scopes),
            engine_native::walk::Buffer::Growing(&mut entries),
            engine_native::walk::Buffer::Growing(&mut names),
            &mut reuse,
        ),
    };
    scopes.truncate(walk.outcome.len());
    entries.truncate(walk.entries_len);
    names.truncate(walk.bytes_len);
    if matches!(walk.outcome, Outcome::Rows { .. }) {
        reuse.finish(true, &scopes, &entries, &names);
    } else {
        reuse.finish(false, &[], &[], &[]);
    }
    let calls = memory.calls.into_inner();
    let node_reads = calls
        .iter()
        .filter(|&&address| {
            (0xd000..0xd000 + 5 * 0x400).contains(&address)
                || (HEAP_KEY..HEAP_KEY + 0x100).contains(&address)
        })
        .count();
    let stores_read = (0..STORES.len())
        .filter(|&store| {
            calls
                .iter()
                .any(|&address| (STORES[store]..STORES[store] + 0x30).contains(&address))
        })
        .collect();
    let character_reads = calls
        .iter()
        .filter(|&&address| (0x5000..0x6400).contains(&address))
        .count();
    Hour {
        walk,
        scopes,
        entries,
        names,
        counts: store.counts(),
        node_reads,
        stores_read,
        character_reads,
    }
}

/// The first hour reads and holds every store; this is what the old
/// behaviour does every hour.
fn first_hour(bytes: &[u8], store: &mut VariableStore, watches: &Watches) -> Hour {
    let first = walk(bytes, store, watches, Check::Nothing);
    assert_eq!(first.walk.outcome, Outcome::Rows { len: 5 });
    assert_eq!(
        (
            first.counts.read,
            first.counts.reused,
            first.counts.read_rows
        ),
        (5, 0, 5)
    );
    assert_ne!(first.node_reads, 0);
    assert_eq!(first.stores_read, [0, 1, 2, 3, 4]);
    assert!(!first.counts.quiet);
    watches.boundary(false);
    first
}

/// The first hour, then the quiet hour that reads once more each store the
/// first walk registered after reading its header and seed.
fn settled(bytes: &[u8], store: &mut VariableStore, watches: &Watches) -> Hour {
    let first = first_hour(bytes, store, watches);
    let settle = walk(bytes, store, watches, Check::Nothing);
    assert!(settle.counts.quiet);
    assert_eq!(
        (
            settle.counts.visited,
            settle.counts.read,
            settle.counts.reused
        ),
        (5, 0, 5)
    );
    assert_eq!(settle.stores_read, [0, 1, 2, 3, 4]);
    assert_eq!(settle.node_reads, 0);
    assert_eq!(settle.published(), first.published());
    watches.boundary(false);
    first
}

fn site(name: &str) -> usize {
    engine_mutations::script_variable_mutations::MUTATIONS
        .iter()
        .position(|(site, _, _)| site.name == name)
        .expect("variable site")
}

/// The fixture scopes' sampling keys.
fn keys() -> [u64; 5] {
    [
        cvariables::Scope::Global,
        cvariables::Scope::Country(2),
        cvariables::Scope::State(1),
        cvariables::Scope::Character(44),
        cvariables::Scope::Character(45),
    ]
    .map(variable_store::key)
}

/// The old behaviour read every scope's identity and every store's vtable,
/// header and seed each hour. A settled quiet hour reads none of them, no
/// node and no character, and still publishes a fresh walk's rows.
#[test]
fn quiet_hours_visit_no_store_and_publish_a_fresh_walks_rows() {
    let bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    let expected = fresh(&bytes);
    for _ in 0..3 {
        let quiet = walk(&bytes, &mut store, &watches, Check::Nothing);
        assert!(quiet.counts.quiet);
        assert_eq!(
            (
                quiet.counts.visited,
                quiet.counts.read,
                quiet.counts.reused,
                quiet.counts.reused_rows
            ),
            (0, 0, 5, 5)
        );
        assert!(quiet.stores_read.is_empty());
        assert_eq!((quiet.node_reads, quiet.character_reads), (0, 0));
        assert_eq!(quiet.published(), expected);
        watches.boundary(false);
    }
}

#[test]
fn a_hooked_write_rereads_only_its_store() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    let first = settled(&bytes, &mut store, &watches);
    // set_variable on country 2: the detour notifies around the write.
    let notification = watches.notify(STORES[1], site("get_mutable_value"));
    set_value(&mut bytes, 1, -5);
    drop(notification);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(hour.counts.quiet);
    assert_eq!(
        (hour.counts.visited, hour.counts.read, hour.counts.reused),
        (1, 1, 4)
    );
    assert_eq!(hour.stores_read, [1]);
    // Only the written store's end sentinel and node block were read.
    assert_eq!(hour.node_reads, 2);
    let mut expected = first.rows();
    expected[1].2[0].1 = -5;
    assert_eq!(hour.rows(), expected);
    assert_eq!(hour.published(), fresh(&bytes));
    // The re-read store is carried again from the next quiet hour.
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!(
        (quiet.counts.visited, quiet.counts.read, quiet.counts.reused),
        (0, 0, 5)
    );
    assert_eq!(quiet.published(), fresh(&bytes));
}

/// A store that gains and then loses a variable moves every later scope's
/// entries and key bytes; the quiet hours still publish a fresh walk's
/// arenas, and so does the hour after, from the held capture they left.
#[test]
fn carried_rows_equal_a_fresh_walk_when_a_store_grows_and_shrinks() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    let insert = watches.notify(STORES[1], site("get_mutable_value"));
    add_variable(&mut bytes, 1, b"second", 42);
    drop(insert);
    let write = watches.notify(STORES[3], site("get_mutable_value"));
    set_value(&mut bytes, 3, 7);
    drop(write);
    let grown = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(grown.counts.quiet);
    assert_eq!((grown.counts.visited, grown.counts.read), (2, 2));
    assert_eq!(grown.walk.entries_len, 6);
    assert_eq!(grown.published(), fresh(&bytes));
    watches.boundary(false);
    let carried = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!(carried.counts.visited, 0);
    assert_eq!(carried.published(), fresh(&bytes));
    watches.boundary(false);
    let erase = watches.notify(STORES[1], site("erase_variable"));
    erase_variable(&mut bytes, 1);
    drop(erase);
    let shrunk = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(shrunk.counts.quiet);
    assert_eq!(shrunk.walk.entries_len, 5);
    assert_eq!(shrunk.published(), fresh(&bytes));
}

/// Scoped variable context end rewrites a store's seed: its observed call
/// marks that store, and the quiet hour publishes the new seed.
#[test]
fn an_ended_context_marks_the_store_whose_seed_it_changed() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    let notification = watches.notify(STORES[1], site("end_effect_execute"));
    set_seed(&mut bytes, 1, 77);
    drop(notification);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(hour.counts.quiet);
    assert_eq!(hour.stores_read, [1]);
    assert_eq!(hour.scopes[1].seed, Some(77));
    assert_eq!(hour.published(), fresh(&bytes));
}

/// A seed no hook saw is carried by quiet hours whose slice misses its
/// store (the premise: every seed writer is observed), and the slice that
/// reads it finds the difference, poisons reuse and walks afresh.
#[test]
fn a_seed_no_hook_saw_is_found_by_the_slice_and_poisons() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    set_seed(&mut bytes, 2, 99);
    let keys = keys();
    let miss = (0..ROTATION as i32)
        .find(|&hour| !keys.iter().any(|&key| Check::Slice { hour }.covers(key)))
        .expect("an hour whose slice holds no fixture scope");
    let stale = walk(&bytes, &mut store, &watches, Check::Slice { hour: miss });
    assert!(stale.counts.quiet);
    assert_eq!(stale.scopes[2].seed, Some(1002));
    watches.boundary(false);
    let hit = (0..ROTATION as i32)
        .find(|&hour| Check::Slice { hour }.covers(keys[2]))
        .expect("the state's slice hour");
    let caught = walk(&bytes, &mut store, &watches, Check::Slice { hour: hit });
    assert!(!caught.counts.quiet);
    assert_eq!(caught.counts.mismatches, 1);
    assert!(watches.poisoned());
    assert_eq!(caught.scopes[2].seed, Some(99));
    assert_eq!(caught.published(), fresh(&bytes));
}

/// A quiet hour's slice that reads entries no hook saw change poisons reuse
/// and the hour walks afresh; every later hour reads every store.
#[test]
fn a_quiet_slice_mismatch_poisons_and_walks_afresh() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    set_value(&mut bytes, 3, 1234);
    let hit = (0..ROTATION as i32)
        .find(|&hour| Check::Slice { hour }.covers(keys()[3]))
        .expect("character 44's slice hour");
    let caught = walk(&bytes, &mut store, &watches, Check::Slice { hour: hit });
    assert!(!caught.counts.quiet);
    assert_eq!(caught.counts.mismatches, 1);
    assert!(watches.poisoned());
    assert_eq!(caught.rows()[3].2[0].1, 1234);
    assert_eq!(caught.published(), fresh(&bytes));
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!after.counts.quiet);
    assert_eq!((after.counts.read, after.counts.reused), (5, 0));
}

/// Any hour that walks every scope reads every store's header and seed: one
/// whose registration still holds and that no take marked, but whose seed
/// moved, was written where no hook saw, and poisons reuse.
#[test]
fn a_walk_finds_a_seed_no_hook_saw() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    set_seed(&mut bytes, 2, 55);
    // A roster write makes the hour walk.
    watches.roster_write();
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!hour.counts.quiet);
    assert_eq!(hour.counts.mismatches, 1);
    assert!(watches.poisoned());
    assert_eq!(hour.scopes[2].seed, Some(55));
    assert_eq!(hour.published(), fresh(&bytes));
}

/// An exception unwinding a seed-changing context ends it through the unwind
/// maps, where no hook sees the seed it writes. The exception handler moves
/// the structure token, so that hour walks every scope and publishes the new
/// seed without counting a missed write; the next hour is quiet again.
#[test]
fn an_exception_walks_every_scope_for_one_hour() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    watches.exception();
    set_seed(&mut bytes, 2, 31);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!hour.counts.quiet);
    assert_eq!(hour.stores_read, [0, 1, 2, 3, 4]);
    assert_eq!((hour.counts.read, hour.counts.reused), (0, 5));
    assert_eq!(hour.counts.mismatches, 0);
    assert!(!watches.poisoned());
    assert_eq!(hour.scopes[2].seed, Some(31));
    assert_eq!(hour.published(), fresh(&bytes));
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(after.counts.quiet);
    assert_eq!(after.counts.visited, 0);
    assert_eq!(after.published(), fresh(&bytes));
}

#[test]
fn an_invalidating_hour_rereads_every_store() {
    let bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    // Load, gap or recovery.
    watches.boundary(true);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!hour.counts.quiet);
    assert_eq!((hour.counts.read, hour.counts.reused), (5, 0));
    assert_eq!(hour.rows(), first.rows());
    // A forgotten store (a failed job) reads every store too.
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!forgotten.counts.quiet);
    assert_eq!((forgotten.counts.read, forgotten.counts.reused), (5, 0));
}

/// A store is destroyed only through the release, which moves the
/// structure token, and created only with its owner: a new roster
/// character moves the roster token, a new state a state slot, a reused
/// country the census. Each such hour walks every scope and publishes a
/// fresh walk's rows; the hour after is quiet again.
#[test]
fn a_store_created_or_destroyed_walks_every_scope() {
    type Change = fn(&mut Vec<u8>, &Watches);
    let changes: [(&str, Change); 4] = [
        ("release", |_, watches| drop(watches.release(STORES[2]))),
        ("roster character", |bytes, watches| {
            let historical = 0x5000 + ccharacter_manager::HISTORICAL_CHARACTERS;
            put(bytes, historical + 8, &3_u32.to_le_bytes());
            put(bytes, historical + 12, &3_u32.to_le_bytes());
            put(bytes, 0x5110, &0x6400_u64.to_le_bytes());
            put(
                bytes,
                0x6400,
                &((BASE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                bytes,
                0x6400 + ccharacter::TYPE,
                &ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(bytes, 0x6400 + ccharacter::ID, &46_u32.to_le_bytes());
            empty_store(bytes, 0x6400 + ccharacter::VARIABLES, 46);
            // The roster append is a hooked roster writer.
            watches.roster_write();
        }),
        ("state slot", |bytes, _| {
            put(
                bytes,
                0x8800,
                &((BASE + cstate::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(bytes, 0x8800 + cstate::ID, &2_u32.to_le_bytes());
            put(bytes, 0x8800 + cstate::VARIABLES, &0xc300_u64.to_le_bytes());
            empty_store(bytes, 0xc300, 2);
            put(bytes, 0x2210, &0x8800_u64.to_le_bytes());
        }),
        ("country census", |bytes, _| {
            // Slot 1's country takes id 1, which the links resolve to it.
            put(bytes, 0x4000 + ccountry::ID, &1_u32.to_le_bytes());
        }),
    ];
    for (name, change) in changes {
        let mut bytes = variable_scopes();
        let watches = Watches::enabled(16);
        let mut store = VariableStore::new();
        settled(&bytes, &mut store, &watches);
        change(&mut bytes, &watches);
        let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
        assert!(!hour.counts.quiet, "{name}");
        assert_eq!(hour.stores_read, [0, 1, 2, 3, 4], "{name}");
        assert_eq!(hour.published(), fresh(&bytes), "{name}");
        assert!(!watches.poisoned(), "{name}");
        watches.boundary(false);
        let after = walk(&bytes, &mut store, &watches, Check::Nothing);
        assert!(after.counts.quiet, "{name}");
        assert_eq!(after.published(), fresh(&bytes), "{name}");
    }
}

#[test]
fn a_changed_header_or_scope_identity_is_read_afresh() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    first_hour(&bytes, &mut store, &watches);
    // Character 44 becomes 46 at the same address and store: only
    // repair_containers writes a rostered id, and the roster domain sees it.
    put(&mut bytes, 0x6000 + ccharacter::ID, &46_u32.to_le_bytes());
    watches.roster_write();
    // The state's storage moves as its table grows under a hooked writer.
    let grow = watches.notify(STORES[2], site("get_mutable_value"));
    let moved = storage(5);
    bytes.copy_within(storage(2)..storage(2) + 0x300, moved);
    put(
        &mut bytes,
        STORES[2] + cvariables::STORAGE,
        &(moved as u64).to_le_bytes(),
    );
    drop(grow);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!hour.counts.quiet);
    assert_eq!((hour.counts.read, hour.counts.reused), (2, 3));
    assert_eq!(hour.scopes[3].scope, cvariables::Scope::Character(46));
    assert_eq!(hour.counts.mismatches, 0);
    assert_eq!(hour.published(), fresh(&bytes));
}

#[test]
fn verify_catches_an_unobserved_write_and_poisons_reuse() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    first_hour(&bytes, &mut store, &watches);
    // Verify of an unchanged hour is never quiet: it reads every carryable
    // store and agrees.
    let agreed = walk(&bytes, &mut store, &watches, Check::Every);
    assert!(!agreed.counts.quiet);
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
    assert_eq!(caught.rows()[2].2[0].1, 9);
    // Without verify, a poisoned domain carries nothing.
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!((after.counts.read, after.counts.reused), (5, 0));
    assert_eq!(after.rows()[2].2[0].1, 9);
}

/// Under verify an hour whose quiet key matched walks, and its scopes are
/// compared with the held walk's: a roster change no hook saw poisons the
/// variable and roster domains.
#[test]
fn verify_compares_a_matched_hours_scopes_with_the_held_walk() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    first_hour(&bytes, &mut store, &watches);
    put(&mut bytes, 0x6000 + ccharacter::ID, &46_u32.to_le_bytes());
    let hour = walk(&bytes, &mut store, &watches, Check::Every);
    assert!(!hour.counts.quiet);
    assert_eq!(hour.counts.mismatches, 1);
    assert!(watches.poisoned() && watches.roster_poisoned());
    assert_eq!(hour.published(), fresh(&bytes));
}

#[test]
fn an_unobserved_write_without_verify_is_carried_stale() {
    // Guards the premise of the family: only hooked writers move a
    // certificate or mark a store, so a missed writer would be carried until
    // the slice, verify or an invalidation, which is why every writer site
    // is hooked.
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    set_value(&mut bytes, 0, 5);
    let stale = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(stale.counts.quiet);
    assert_eq!(stale.counts.reused, 5);
    assert_eq!(stale.rows(), first.rows());
}

#[test]
fn a_failed_walk_holds_nothing() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    first_hour(&bytes, &mut store, &watches);
    let mut broken = bytes.clone();
    put(
        &mut broken,
        0x8000 + cstate::VARIABLES,
        &0_u64.to_le_bytes(),
    );
    // A roster write makes the hour walk, and the walk fails.
    watches.roster_write();
    let failed = walk(&broken, &mut store, &watches, Check::Nothing);
    assert!(matches!(failed.walk.outcome, Outcome::Unavailable { .. }));
    // The failed walk's carried rows certify nothing afterwards.
    set_value(&mut bytes, 0, 5);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(!hour.counts.quiet);
    assert_eq!((hour.counts.read, hour.counts.reused), (5, 0));
    assert_eq!(hour.rows()[0].2[0].1, 5);
}

#[test]
fn an_hourly_slice_verifies_only_its_stores_and_carries_the_rest() {
    let bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    first_hour(&bytes, &mut store, &watches);
    let keys = keys();
    let mut verified = 0;
    for hour in 0..ROTATION as i32 {
        let check = Check::Slice { hour };
        let sampled = walk(&bytes, &mut store, &watches, check);
        let due = keys.iter().filter(|&&key| check.covers(key)).count() as u64;
        assert!(sampled.counts.quiet);
        assert_eq!(
            (sampled.counts.verified, sampled.counts.reused),
            (due, 5 - due)
        );
        assert_eq!(sampled.counts.mismatches, 0);
        assert_eq!(sampled.published(), fresh(&bytes));
        if hour > 0 {
            // Only the slice is visited once the first walk's stores are
            // settled.
            assert_eq!(sampled.counts.visited, due);
        }
        verified += due;
        watches.boundary(false);
    }
    // One rotation compares every carried store exactly once.
    assert_eq!(verified, 5);
}

/// A renewed quiet hour whose visited stores kept their entries' places
/// writes only those stores into the held capture, and one whose store
/// grew copies the whole capture; either way the held capture is the
/// published one, so the next quiet hour publishes a fresh walk's arenas.
#[test]
fn a_renewed_quiet_hour_patches_only_its_visited_stores_in_place() {
    let mut bytes = variable_scopes();
    let watches = Watches::enabled(16);
    let mut store = VariableStore::new();
    settled(&bytes, &mut store, &watches);
    let copies = store.full_copies();
    let write = watches.notify(STORES[1], site("get_mutable_value"));
    set_value(&mut bytes, 1, -5);
    drop(write);
    let written = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(written.counts.quiet);
    assert_eq!(written.published(), fresh(&bytes));
    // The value changed in place: no whole copy.
    assert_eq!(store.full_copies(), copies);
    watches.boundary(false);
    let carried = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!(carried.counts.visited, 0);
    assert_eq!(carried.published(), fresh(&bytes));
    watches.boundary(false);
    let insert = watches.notify(STORES[1], site("get_mutable_value"));
    add_variable(&mut bytes, 1, b"second", 42);
    drop(insert);
    let grown = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert!(grown.counts.quiet);
    assert_eq!(grown.published(), fresh(&bytes));
    // Every later scope moved: the whole capture is copied.
    assert_eq!(store.full_copies(), copies + 1);
    watches.boundary(false);
    let carried = walk(&bytes, &mut store, &watches, Check::Nothing);
    assert_eq!(carried.counts.visited, 0);
    assert_eq!(carried.published(), fresh(&bytes));
}
