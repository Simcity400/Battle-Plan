use super::*;
use crate::memory::SliceMemory;

const BASE: usize = 0x1400_0000;
const CONTAINER: usize = BASE + 0x100;
const STORAGE_ADDRESS: usize = BASE + 0x200;
const TAIL: usize = 5;
const SLOTS: usize = 8 + TAIL;

fn fixture(count: u32) -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    put64(&mut bytes, 0x100, (BASE + VTABLE_RVA) as u64);
    put64(&mut bytes, 0x118, STORAGE_ADDRESS as u64);
    put32(&mut bytes, 0x120, count);
    put32(&mut bytes, 0x124, 7);
    bytes[0x128] = TAIL as u8;
    bytes[0x200 + SLOTS * variable_hash_node::STRIDE + variable_hash_node::DISTANCE] = 0xff;
    bytes
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn node(bytes: &mut [u8], index: usize, hash: u32, key: &[u8], value: i64) {
    let offset = 0x200 + index * variable_hash_node::STRIDE;
    put32(bytes, offset, hash);
    bytes[offset + variable_hash_node::DISTANCE] = (index - (hash & 7) as usize + 1) as u8;
    put64(bytes, offset + 0x18, key.len() as u64);
    if key.len() <= 15 {
        bytes[offset + variable_hash_node::KEY..offset + variable_hash_node::KEY + key.len()]
            .copy_from_slice(key);
        put64(bytes, offset + 0x20, 15);
    } else {
        put64(
            bytes,
            offset + variable_hash_node::KEY,
            (BASE + 0x900) as u64,
        );
        bytes[0x900..0x900 + key.len()].copy_from_slice(key);
        put64(bytes, offset + 0x20, key.len() as u64);
    }
    put64(bytes, offset + variable_hash_node::VALUE, value as u64);
}

fn run(bytes: &[u8], output: &mut [Row], names: &mut [u8]) -> NameWalk {
    walk(
        &SliceMemory { base: BASE, bytes },
        BASE,
        CONTAINER,
        output,
        names,
    )
}

#[test]
fn growing_variable_rows_and_key_bytes_capture_first_and_later_mod_sized_maps() {
    let mut small = fixture(2);
    node(&mut small, 0, 0, b"a", i64::MIN);
    node(&mut small, 1, 1, b"b", i64::MAX);
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let mut expected_rows = [Row::default(); 2];
    let mut expected_names = [0; 2];
    let expected = run(&small, &mut expected_rows, &mut expected_names);
    assert_eq!(expected.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        walk_growing(
            &SliceMemory {
                base: BASE,
                bytes: &small
            },
            BASE,
            CONTAINER,
            &mut rows,
            &mut names
        ),
        expected
    );
    assert_eq!(&rows[..2], &expected_rows);
    assert_eq!(&names[..2], &expected_names);
    const COUNT: usize = 5_000;
    const BUCKETS: usize = 8_192;
    const TAIL: usize = 15;
    let mut bytes = fixture(COUNT as u32);
    bytes.resize(0x200 + (BUCKETS + TAIL + 1) * variable_hash_node::STRIDE, 0);
    bytes[0x200..].fill(0);
    put32(&mut bytes, 0x124, BUCKETS as u32 - 1);
    bytes[0x128] = TAIL as u8;
    bytes[0x200 + (BUCKETS + TAIL) * variable_hash_node::STRIDE + variable_hash_node::DISTANCE] =
        0xff;
    for index in 0..COUNT {
        let at = 0x200 + index * variable_hash_node::STRIDE;
        let key = format!("key_{index:04}");
        put32(&mut bytes, at, index as u32);
        bytes[at + variable_hash_node::DISTANCE] = 1;
        bytes[at + variable_hash_node::KEY..at + variable_hash_node::KEY + key.len()]
            .copy_from_slice(key.as_bytes());
        put64(&mut bytes, at + 0x18, key.len() as u64);
        put64(&mut bytes, at + 0x20, 15);
        put64(&mut bytes, at + variable_hash_node::VALUE, index as u64);
    }
    let mut expected_rows = vec![Row::default(); COUNT];
    let mut expected_names = vec![0; COUNT * 8];
    let expected = run(&bytes, &mut expected_rows, &mut expected_names);
    assert_eq!(expected.outcome, Outcome::Rows { len: COUNT });
    assert_eq!(
        (expected.bytes_len, expected.required_bytes),
        (COUNT * 8, COUNT * 8)
    );
    assert_eq!(
        walk_growing(
            &SliceMemory {
                base: BASE,
                bytes: &bytes
            },
            BASE,
            CONTAINER,
            &mut rows,
            &mut names
        ),
        expected
    );
    assert_eq!(&rows[..COUNT], expected_rows.as_slice());
    assert_eq!(&names[..expected.bytes_len], expected_names.as_slice());
}

#[test]
fn growing_variable_scopes_extend_all_three_buffers_on_the_first_pass() {
    let bytes = scope_fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut expected_scopes = [ScopeRow::default(); 5];
    let mut expected_entries = [Row::default(); 4];
    let mut expected_names = [0; 6];
    let expected = walk_scopes(
        &memory,
        BASE,
        BASE + 0x1000,
        &mut expected_scopes,
        &mut expected_entries,
        &mut expected_names,
    );
    assert_eq!(expected.outcome, Outcome::Rows { len: 5 });
    let mut scopes = Vec::new();
    let mut entries = Vec::new();
    let mut names = Vec::new();
    assert_eq!(
        walk_scopes_growing(
            &memory,
            BASE,
            BASE + 0x1000,
            &mut scopes,
            &mut entries,
            &mut names
        ),
        expected
    );
    assert_eq!(&scopes[..5], &expected_scopes);
    assert_eq!(&entries[..4], &expected_entries);
    assert_eq!(&names[..6], &expected_names);
}

#[test]
fn holes_probe_overflow_and_literal_bytes_preserve_exact_values() {
    let mut bytes = fixture(3);
    node(&mut bytes, 2, 2, b"", i64::MIN);
    node(&mut bytes, 7, 7, b"\xff\0x", -100_001);
    node(&mut bytes, 8, 7, b"long_variable_key_bytes", i64::MAX);
    // A plausible stale string in an unoccupied slot must not create a row.
    let stale = 0x200 + variable_hash_node::STRIDE;
    bytes[stale + variable_hash_node::KEY..stale + variable_hash_node::KEY + 3]
        .copy_from_slice(b"old");
    put64(&mut bytes, stale + 0x18, 3);
    put64(&mut bytes, stale + 0x20, 15);
    let mut output = [Row::default(); 3];
    let mut names = [0; 64];
    let result = run(&bytes, &mut output, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!(result.bytes_len, 26);
    assert_eq!(&names[..3], b"\xff\0x");
    assert_eq!(&names[3..26], b"long_variable_key_bytes");
    assert_eq!(
        output.map(|row| row.value_raw),
        [i64::MIN, -100_001, i64::MAX]
    );
    assert_eq!(output[0].name_len, 0);
}

#[test]
fn capacity_omits_whole_rows_and_reports_required_key_bytes() {
    let mut bytes = fixture(3);
    node(&mut bytes, 0, 0, b"alpha", 1);
    node(&mut bytes, 1, 1, b"long_variable_key_bytes", 2);
    node(&mut bytes, 2, 2, b"z", 3);
    let mut output = [Row::default(); 3];
    let mut names = [0; 6];
    let result = run(&bytes, &mut output, &mut names);
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 2,
            unread_rows: 1
        }
    );
    assert_eq!(result.required_bytes, 29);
    assert_eq!(result.bytes_len, 6);
    assert_eq!(&names, b"alphaz");
    let result = run(&bytes, &mut [], &mut []);
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
    assert_eq!(result.required_bytes, 29);
}

#[test]
fn declared_count_probe_law_and_end_sentinel_are_all_checked() {
    let mut bytes = fixture(1);
    node(&mut bytes, 3, 3, b"x", 7);
    let original = bytes.clone();
    let mut output = [Row::default(); 2];
    let mut names = [0; 16];
    for (offset, replacement) in [
        (0x120, 0),
        (0x120, 2),
        (0x124, 6),
        (0x128, 4),
        (
            0x200 + SLOTS * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            0,
        ),
        (
            0x200 + 3 * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            2,
        ),
        (
            0x200 + 3 * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            0xff,
        ),
    ] {
        bytes.copy_from_slice(&original);
        bytes[offset] = replacement;
        assert!(matches!(
            run(&bytes, &mut output, &mut names).outcome,
            Outcome::Unavailable { .. }
        ));
    }
}

#[test]
fn empty_shared_storage_needs_no_node_memory() {
    let mut bytes = fixture(0);
    put64(&mut bytes, 0x118, (BASE + EMPTY_STORAGE_RVA) as u64);
    put32(&mut bytes, 0x124, 0);
    bytes[0x128] = 0;
    assert_eq!(
        run(&bytes[..0x129], &mut [], &mut []).outcome,
        Outcome::Rows { len: 0 }
    );
    put32(&mut bytes, 0x120, 1);
    assert!(matches!(
        run(&bytes, &mut [], &mut []).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scoped_walk_keeps_alias_identity_literal_keys_and_authenticated_empty_scopes() {
    let mut bytes = scope_fixture();
    let seeds = [0, u32::MAX, 1, 0x8000_0000, 9];
    for (container, seed) in [
        0xa000,
        0xb000,
        0xc000,
        0x8000 + ccharacter::VARIABLES,
        0x9000 + ccharacter::VARIABLES,
    ]
    .into_iter()
    .zip(seeds)
    {
        put32(&mut bytes, container + SEED, seed);
    }
    let mut scopes = [ScopeRow::default(); 5];
    let mut entries = [Row::default(); 4];
    let mut names = [0; 6];
    let captured = walk_scopes(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        BASE,
        BASE + 0x1000,
        &mut scopes,
        &mut entries,
        &mut names,
    );
    assert_eq!(
        captured,
        ScopeWalk {
            outcome: Outcome::Rows { len: 5 },
            entries_len: 4,
            required_entries: 4,
            bytes_len: 6,
            required_bytes: 6
        }
    );
    // The characters' stores are embedded; 45's is the empty sentinel.
    assert_eq!(
        scopes.map(|row| row.scope),
        [
            Scope::Global,
            Scope::Country(2),
            Scope::State(1),
            Scope::Character(44),
            Scope::Character(45)
        ]
    );
    assert_eq!(scopes[2].entries_len, 0);
    assert_eq!(scopes[4].entries_len, 0);
    assert_eq!(scopes.map(|row| row.seed), seeds.map(Some));
    assert_eq!(
        entries.map(|row| row.value_raw),
        [i64::MIN, i64::MAX, -100001, 300_000]
    );
    assert_eq!(names, [b'g', b'g', 0xff, 0, b'x', b'c']);
    assert_eq!(entries.map(|row| row.name_start), [0, 1, 2, 5]);
}

#[test]
fn scope_capacity_omits_whole_scopes_and_bad_later_scope_invalidates_everything() {
    let mut bytes = scope_fixture();
    let mut scopes = [ScopeRow::default(); 5];
    let mut entries = [Row::default(); 4];
    let mut names = [0; 6];
    for small_names in [false, true] {
        let entry_capacity = if small_names { 3 } else { 2 };
        let name_capacity = if small_names { 3 } else { 5 };
        let captured = walk_scopes(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            BASE,
            BASE + 0x1000,
            &mut scopes,
            &mut entries[..entry_capacity],
            &mut names[..name_capacity],
        );
        // The country's two keys do not fit; later scopes still do.
        assert_eq!(
            captured,
            ScopeWalk {
                outcome: Outcome::Truncated {
                    len: 4,
                    unread_rows: 1
                },
                entries_len: 2,
                required_entries: 4,
                bytes_len: 2,
                required_bytes: 6
            }
        );
        assert_eq!(
            scopes[..4].iter().map(|row| row.scope).collect::<Vec<_>>(),
            [
                Scope::Global,
                Scope::State(1),
                Scope::Character(44),
                Scope::Character(45)
            ]
        );
    }
    let captured = walk_scopes(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        BASE,
        BASE + 0x1000,
        &mut [],
        &mut [],
        &mut [],
    );
    assert_eq!(
        captured.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 5
        }
    );
    assert_eq!((captured.required_entries, captured.required_bytes), (4, 6));
    put64(&mut bytes, 0x7000 + cstate::VARIABLES, 0);
    let captured = walk_scopes(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        BASE,
        BASE + 0x1000,
        &mut scopes,
        &mut entries,
        &mut names,
    );
    assert_eq!(
        captured.outcome,
        Outcome::Unavailable {
            reason: "script variable scope root is null"
        }
    );
    assert_eq!((captured.entries_len, captured.bytes_len), (0, 0));
}

fn scope_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xd000];
    for (offset, data, count) in [
        (cgame_state::COUNTRIES, 0x2000, 3),
        (cgame_state::COUNTRY_LINKS, 0x2100, 3),
        (cgame_state::STATES, 0x2200, 3),
    ] {
        put64(&mut bytes, 0x1000 + offset, (BASE + data) as u64);
        put32(&mut bytes, 0x1000 + offset + 8, count);
        put32(&mut bytes, 0x1000 + offset + 12, count);
    }
    for slot in [1, 2] {
        put64(&mut bytes, 0x2000 + slot * 8, (BASE + 0x4000) as u64);
        put32(&mut bytes, 0x2100 + slot * 4, 1);
    }
    put64(&mut bytes, 0x4000, (BASE + ccountry::VTABLE_RVA) as u64);
    put32(&mut bytes, 0x4000 + ccountry::ID, 2);
    put64(&mut bytes, 0x2208, (BASE + 0x7000) as u64);
    put64(&mut bytes, 0x7000, (BASE + cstate::VTABLE_RVA) as u64);
    put32(&mut bytes, 0x7000 + cstate::ID, 1);
    for (root, offset, container, count) in [
        (0x1000, cgame_state::GLOBAL_VARIABLES, 0xa000, 1),
        (0x4000, ccountry::VARIABLES, 0xb000, 2),
        (0x7000, cstate::VARIABLES, 0xc000, 0),
    ] {
        let mut block = fixture(count);
        if count != 0 {
            node(
                &mut block,
                0,
                0,
                b"g",
                if count == 1 { i64::MIN } else { i64::MAX },
            );
        }
        if count == 2 {
            node(&mut block, 1, 1, b"\xff\0x", -100001);
        }
        bytes[container - 0x100..container + 0xf00].copy_from_slice(&block);
        put64(&mut bytes, root + offset, (BASE + container) as u64);
        put64(
            &mut bytes,
            container + STORAGE,
            (BASE + container + 0x100) as u64,
        );
    }
    // Two historical characters: 44 with one variable, 45 with the empty
    // sentinel. Their stores are embedded at +0xD8, not pointed to.
    let container = 0x8000 + ccharacter::VARIABLES;
    let mut block = fixture(1);
    node(&mut block, 0, 0, b"c", 300_000);
    bytes[container - 0x100..container + 0xf00].copy_from_slice(&block);
    put64(
        &mut bytes,
        container + STORAGE,
        (BASE + container + 0x100) as u64,
    );
    let empty = 0x9000 + ccharacter::VARIABLES;
    put64(&mut bytes, empty, (BASE + VTABLE_RVA) as u64);
    put64(
        &mut bytes,
        empty + STORAGE,
        (BASE + EMPTY_STORAGE_RVA) as u64,
    );
    put64(
        &mut bytes,
        0x1000 + cgame_state::CHARACTER_MANAGER,
        (BASE + 0x5000) as u64,
    );
    put64(
        &mut bytes,
        0x5000,
        (BASE + ccharacter_manager::VTABLE_RVA) as u64,
    );
    let historical = 0x5000 + ccharacter_manager::HISTORICAL_CHARACTERS;
    put64(&mut bytes, historical, (BASE + 0x5100) as u64);
    put32(&mut bytes, historical + 8, 2);
    put32(&mut bytes, historical + 12, 2);
    for (index, (character, id)) in [(0x8000_usize, 44_u32), (0x9000, 45)]
        .into_iter()
        .enumerate()
    {
        put64(&mut bytes, 0x5100 + index * 8, (BASE + character) as u64);
        put64(
            &mut bytes,
            character,
            (BASE + ccharacter::VTABLE_RVA) as u64,
        );
        put32(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE,
        );
        put32(&mut bytes, character + ccharacter::ID, id);
    }
    bytes
}

#[test]
fn bad_names_and_faults_do_not_publish_a_partial_scope() {
    let mut bytes = fixture(2);
    node(&mut bytes, 0, 0, b"good", 7);
    node(&mut bytes, 1, 1, b"long_variable_key_bytes", 8);
    let mut output = [Row::default(); 2];
    let mut names = [0; 64];
    let failed = run(&bytes[..0x900], &mut output, &mut names);
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
    assert_eq!((failed.bytes_len, failed.required_bytes), (0, 0));
    bytes[0x900 + 23] = 1;
    assert!(matches!(
        run(&bytes, &mut output, &mut names).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scoped_seed_failure_preserves_entries_and_distinguishes_unknown_from_zero() {
    struct SeedFault<'a>(SliceMemory<'a>);
    impl Memory for SeedFault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == BASE + 0xb000 + SEED && destination.len() == 4 {
                false
            } else {
                self.0.copy(address, destination)
            }
        }
    }
    let bytes = scope_fixture();
    let mut scopes = [ScopeRow::default(); 5];
    let mut entries = [Row::default(); 4];
    let mut names = [0; 6];
    let captured = walk_scopes(
        &SeedFault(SliceMemory {
            base: BASE,
            bytes: &bytes,
        }),
        BASE,
        BASE + 0x1000,
        &mut scopes,
        &mut entries,
        &mut names,
    );
    assert_eq!(captured.outcome, Outcome::Rows { len: 5 });
    assert_eq!(captured.entries_len, 4);
    assert_eq!(scopes[0].seed, Some(0));
    assert_eq!(scopes[1].seed, None);
    assert_eq!(scopes[1].entries_len, 2);
    assert_eq!(entries[1].value_raw, i64::MAX);
    assert_eq!(names, [b'g', b'g', 0xff, 0, b'x', b'c']);
}

#[test]
fn scoped_seeds_include_all_5000_country_stores_without_entry_demand() {
    use crate::ccountry::synthetic::World;
    const MODULE: usize = 0x4000_0000;
    const COUNTRIES: u32 = 5_000;
    let mut world = World::new(MODULE, COUNTRIES, ccountry::VARIABLES + 8);
    let game = world.game_state();
    let empty_store = |world: &mut World, seed| {
        let container = world.alloc(0x40);
        world.u64(container, (MODULE + VTABLE_RVA) as u64);
        world.u32(container + SEED, seed);
        world.u64(container + STORAGE, (MODULE + EMPTY_STORAGE_RVA) as u64);
        container
    };
    let global = empty_store(&mut world, u32::MAX);
    world.u64(game + cgame_state::GLOBAL_VARIABLES, global as u64);
    for slot in 1..=COUNTRIES {
        let container = empty_store(&mut world, slot);
        let country = world.object(slot);
        world.u64(country + ccountry::VARIABLES, container as u64);
    }
    let pointers = world.alloc(16);
    let state = world.alloc(cstate::VARIABLES + 8);
    world.u64(state, (MODULE + cstate::VTABLE_RVA) as u64);
    world.u32(state + cstate::ID, 1);
    let container = empty_store(&mut world, 0);
    world.u64(state + cstate::VARIABLES, container as u64);
    world.u64(pointers + 8, state as u64);
    world.array(game + cgame_state::STATES, pointers, 2);
    let manager = world.alloc(0x80);
    world.u64(manager, (MODULE + ccharacter_manager::VTABLE_RVA) as u64);
    world.u64(game + cgame_state::CHARACTER_MANAGER, manager as u64);
    let memory = world.memory();
    let expected = COUNTRIES as usize + 2;
    let mut scopes = vec![ScopeRow::default(); expected];
    let captured = walk_scopes(&memory, MODULE, game, &mut scopes, &mut [], &mut []);
    assert_eq!(captured.outcome, Outcome::Rows { len: expected });
    assert_eq!((captured.required_entries, captured.required_bytes), (0, 0));
    assert_eq!(scopes[0].seed, Some(u32::MAX));
    for slot in 1..=COUNTRIES {
        assert_eq!(scopes[slot as usize].scope, Scope::Country(slot));
        assert_eq!(scopes[slot as usize].seed, Some(slot));
    }
    assert_eq!(scopes[expected - 1].seed, Some(0));
    let mut growing_scopes = Vec::new();
    let mut growing_entries = Vec::new();
    let mut growing_names = Vec::new();
    assert_eq!(
        walk_scopes_growing(
            &memory,
            MODULE,
            game,
            &mut growing_scopes,
            &mut growing_entries,
            &mut growing_names
        ),
        captured
    );
    assert_eq!(&growing_scopes[..expected], scopes.as_slice());
    let demand = walk_scopes(&memory, MODULE, game, &mut [], &mut [], &mut []);
    assert_eq!(
        demand.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: expected as u32,
        }
    );
}

/// Capture cost at scale (run with `--ignored --nocapture`): every
/// country with ten variables, 1,000 states with two each, and no or ten
/// characters per country, one in ten holding a variable, the rest the
/// empty sentinel.
#[test]
#[ignore = "bench"]
fn bench_script_variable_scopes() {
    use crate::ccountry::synthetic::World;
    const MODULE: usize = 0x4000_0000;
    const STATES: u32 = 1_000;
    // One store of `count` short keys, two buckets per key.
    fn store(world: &mut World, container: usize, count: u32, seed: u32) {
        world.u64(container, (MODULE + VTABLE_RVA) as u64);
        world.u32(container + SEED, seed);
        if count == 0 {
            world.u64(container + STORAGE, (MODULE + EMPTY_STORAGE_RVA) as u64);
            return;
        }
        let buckets = (count * 2).next_power_of_two();
        let tail = buckets.trailing_zeros() + 2;
        let slots = (buckets + tail) as usize;
        let storage = world.alloc((slots + 1) * variable_hash_node::STRIDE);
        world.u64(container + STORAGE, storage as u64);
        world.u32(container + ENTRY_COUNT, count);
        world.u32(container + BUCKET_MASK, buckets - 1);
        world.put(container + PROBE_TAIL, &[tail as u8]);
        world.put(
            storage + slots * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            &[0xff],
        );
        for key in 0..count {
            let home = (key * 2) as usize;
            let node = storage + home * variable_hash_node::STRIDE;
            world.u32(node, home as u32);
            world.put(node + variable_hash_node::DISTANCE, &[1]);
            let name = format!("var_{seed}_{key}");
            world.put(node + variable_hash_node::KEY, name.as_bytes());
            world.u64(node + 0x18, name.len() as u64);
            world.u64(node + 0x20, 15);
            world.u64(node + variable_hash_node::VALUE, u64::from(key) * 100_000);
        }
    }
    for (countries, per_country) in [(1_500_u32, 0), (1_500, 10), (5_000, 0), (5_000, 10)] {
        let mut world = World::new(MODULE, countries, cstate::VARIABLES + 8);
        let game = world.game_state();
        let global = world.alloc(0x40);
        store(&mut world, global, 4, 0);
        world.u64(game + cgame_state::GLOBAL_VARIABLES, global as u64);
        for slot in 1..=countries {
            let container = world.alloc(0x40);
            store(&mut world, container, 10, slot);
            let object = world.object(slot);
            world.u64(object + ccountry::VARIABLES, container as u64);
        }
        let states = world.alloc((STATES as usize + 1) * 8);
        world.array(game + cgame_state::STATES, states, STATES + 1);
        for id in 1..=STATES {
            let state = world.alloc(cstate::VARIABLES + 8);
            world.u64(state, (MODULE + cstate::VTABLE_RVA) as u64);
            world.u32(state + cstate::ID, id);
            let container = world.alloc(0x40);
            store(&mut world, container, 2, id);
            world.u64(state + cstate::VARIABLES, container as u64);
            world.u64(states + id as usize * 8, state as u64);
        }
        let characters = countries * per_country;
        let manager = world.alloc(0x80);
        world.u64(game + cgame_state::CHARACTER_MANAGER, manager as u64);
        world.u64(manager, (MODULE + ccharacter_manager::VTABLE_RVA) as u64);
        let pointers = world.alloc(characters as usize * 8);
        world.array(
            manager + ccharacter_manager::HISTORICAL_CHARACTERS,
            pointers,
            characters,
        );
        for index in 0..characters {
            let character = world.alloc(ccharacter::VARIABLES + 0x40);
            world.u64(character, (MODULE + ccharacter::VTABLE_RVA) as u64);
            world.u32(character + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
            world.u32(character + ccharacter::ID, index + 1);
            let held = u32::from(index % 10 == 0);
            store(&mut world, character + ccharacter::VARIABLES, held, index);
            world.u64(pointers + index as usize * 8, character as u64);
        }
        let memory = world.memory();
        let scopes_len = 1 + countries as usize + STATES as usize + characters as usize;
        let entries_len =
            4 + countries as usize * 10 + STATES as usize * 2 + characters.div_ceil(10) as usize;
        let mut scopes = vec![ScopeRow::default(); scopes_len];
        let mut entries = vec![Row::default(); entries_len];
        let mut names = vec![0_u8; entries_len * 16];
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let begin = std::time::Instant::now();
            let walk = walk_scopes(&memory, MODULE, game, &mut scopes, &mut entries, &mut names);
            best = best.min(begin.elapsed());
            assert_eq!(walk.outcome, Outcome::Rows { len: scopes_len });
            assert_eq!(walk.entries_len, entries_len);
        }
        println!(
            "script variable scopes, {countries} countries, {STATES} states, {characters} characters, {entries_len} entries: best of 5 {best:?}"
        );
    }
}
