use super::*;
use engine_native::ccountry::synthetic::World;

const MAP_SLOTS: usize = 5;

struct Fixture {
    world: World,
    manager: usize,
    vector: usize,
    objects: [usize; 2],
    trade: usize,
    pressure: usize,
}

fn map(world: &mut World, map: usize, data: usize, count: i32, stride: usize) {
    world.u64(map + operation_token_map::TABLE, data as u64);
    world.u32(map + operation_token_map::COUNT, count as u32);
    world.u32(map + operation_token_map::MASK, 1);
    world.put(map + operation_token_map::OVERFLOW, &[3]);
    world.put(data + MAP_SLOTS * stride + ENTRY_PROBE, &[u8::MAX]);
}

fn entry(world: &mut World, slot: usize, hash: u32, probe: u8, country: u32, words: &[i64]) {
    world.u32(slot + ENTRY_HASH, hash);
    world.put(slot + ENTRY_PROBE, &[probe]);
    world.u32(slot + ENTRY_COUNTRY, country);
    for (index, word) in words.iter().enumerate() {
        world.u64(slot + TRADE_VALUE + index * 8, *word as u64);
    }
}

fn fixture() -> Fixture {
    let mut world = World::new(0, 3, 0x10);
    let manager = world.alloc(0x40);
    let vector = world.alloc(4 * 8);
    let first = world.alloc(strategic::SIZE);
    let second = world.alloc(strategic::SIZE);
    let trade = world.alloc(MAP_SLOTS * TRADE_STRIDE + 8);
    let pressure = world.alloc(MAP_SLOTS * PRESSURE_STRIDE + 8);
    let state = world.game_state();
    world.u64(
        state + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
        manager as u64,
    );
    world.array(manager + manager::OPERATIVES_DATA, vector, 4);
    for (slot, object) in [(1_usize, first), (2, second)] {
        world.u64(vector + slot * 8, object as u64);
        world.u64(object, strategic::VTABLE_RVA.0 as u64);
        world.u32(object + strategic::OWNER, slot as u32);
    }
    world.u64(first + strategic::MISSION_ACTIVITY, 123_456);
    world.u64(first + strategic::OPERATION_ACTIVITY, (-50_000_i64) as u64);
    map(
        &mut world,
        first + strategic::TRADE_INFLUENCE,
        trade,
        2,
        TRADE_STRIDE,
    );
    entry(&mut world, trade, 0, 1, 2, &[250_000]);
    entry(&mut world, trade + TRADE_STRIDE, 2, 2, 3, &[0]);
    map(
        &mut world,
        first + strategic::DIPLOMATIC_PRESSURE,
        pressure,
        1,
        PRESSURE_STRIDE,
    );
    entry(
        &mut world,
        pressure + PRESSURE_STRIDE,
        1,
        1,
        2,
        &[32_768, -16_384],
    );
    world.u64(
        second + strategic::TRADE_INFLUENCE + operation_token_map::TABLE,
        0xdead_0000,
    );
    Fixture {
        world,
        manager,
        vector,
        objects: [first, second],
        trade,
        pressure,
    }
}

fn run(fixture: &Fixture, rows: &mut Vec<Row>, entries: &mut Vec<Entry>) -> Walk {
    walk(
        &fixture.world.memory(),
        0,
        fixture.world.game_state(),
        rows,
        entries,
    )
}

fn refusal(fixture: &Fixture) -> Outcome {
    run(
        fixture,
        &mut vec![Row::default(); 4],
        &mut vec![Entry::default(); 8],
    )
    .outcome
}

#[test]
fn every_slot_reads_its_activity_and_both_maps_in_table_order() {
    let fixture = fixture();
    let mut rows = vec![Row::default(); 3];
    let mut entries = vec![Entry::default(); 4];
    let read = run(&fixture, &mut rows, &mut entries);
    assert_eq!(read.outcome, Outcome::Rows { len: 2 });
    assert_eq!((read.entries_len, read.required_entries), (3, 3));
    assert_eq!(
        rows[..2],
        [
            Row {
                mission_counts: [0; 5],
                native_pointer: fixture.objects[0],
                country: 1,
                slot: 1,
                mission_activity: 123_456,
                operation_activity: -50_000,
                entries_start: 0,
                trade_len: 2,
                pressure_len: 1,
            },
            Row {
                native_pointer: fixture.objects[1],
                country: 2,
                slot: 2,
                entries_start: 3,
                ..Row::default()
            },
        ]
    );
    assert_eq!(
        entries[..3],
        [
            Entry {
                operative: 0,
                owner: 2,
                value: 250_000,
                tension: 0,
            },
            Entry {
                operative: 0,
                owner: 3,
                value: 0,
                tension: 0,
            },
            Entry {
                operative: 0,
                owner: 2,
                value: 32_768,
                tension: -16_384,
            },
        ]
    );
}

#[test]
fn a_null_manager_or_slot_has_no_rows() {
    let mut fixture = fixture();
    let vector = fixture.vector;
    fixture.world.u64(vector + 2 * 8, 0);
    let mut rows = vec![Row::default(); 3];
    let read = run(&fixture, &mut rows, &mut vec![Entry::default(); 4]);
    assert_eq!(read.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].slot, 1);
    let state = fixture.world.game_state();
    fixture
        .world
        .u64(state + cgame_state::STRATEGIC_OPERATIVE_MANAGER, 0);
    assert_eq!(refusal(&fixture), Outcome::Rows { len: 0 });
}

#[test]
fn the_count_probe_counts_whole_rows_with_exact_entry_demand_and_storage_grows() {
    let fixture = fixture();
    let sizing = count(&fixture.world.memory(), 0, fixture.world.game_state());
    assert_eq!(
        sizing.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(sizing.required_entries, 3);
    let (mut rows, mut entries) = (Vec::new(), Vec::new());
    let grown = walk(
        &fixture.world.memory(),
        0,
        fixture.world.game_state(),
        &mut rows,
        &mut entries,
    );
    assert_eq!(grown.outcome, Outcome::Rows { len: 2 });
    assert_eq!(grown.entries_len, 3);
}

#[test]
fn object_and_map_refusals_are_named() {
    let check = |change: &dyn Fn(&mut Fixture), reason: &'static str| {
        let mut fixture = fixture();
        change(&mut fixture);
        assert_eq!(
            refusal(&fixture),
            Outcome::Unavailable { reason },
            "{reason}"
        );
    };
    check(
        &|f| f.world.u64(f.objects[0], 0),
        "strategic operative vtable mismatch",
    );
    check(
        &|f| f.world.u32(f.objects[0] + strategic::OWNER, 2),
        "strategic operative owner does not match its country slot",
    );
    check(
        &|f| {
            f.world
                .array(f.manager + manager::OPERATIVES_DATA, f.vector, 2)
        },
        "strategic operative slot unreadable or outside the vector",
    );
    check(
        &|f| f.world.put(f.trade + TRADE_STRIDE + ENTRY_PROBE, &[1]),
        "strategic operative map probe distance invalid",
    );
    check(
        &|f| {
            f.world.u32(
                f.objects[0] + strategic::TRADE_INFLUENCE + operation_token_map::COUNT,
                1,
            )
        },
        "strategic operative map census differs from count",
    );
    check(
        &|f| {
            f.world
                .put(f.pressure + MAP_SLOTS * PRESSURE_STRIDE + ENTRY_PROBE, &[0])
        },
        "strategic operative map end sentinel invalid or unreadable",
    );
    check(
        &|f| {
            f.world.put(
                f.objects[0] + strategic::DIPLOMATIC_PRESSURE + operation_token_map::OVERFLOW,
                &[4],
            )
        },
        "strategic operative map geometry invalid",
    );
    check(
        &|f| {
            f.world.u32(
                f.objects[1] + strategic::TRADE_INFLUENCE + operation_token_map::COUNT,
                1,
            )
        },
        "strategic operative static map has live entries",
    );
    check(
        &|f| {
            f.world.u32(
                f.objects[1] + strategic::TRADE_INFLUENCE + operation_token_map::COUNT,
                u32::MAX,
            )
        },
        "strategic operative map count negative",
    );
    check(
        &|f| {
            f.world
                .put(f.trade + 3 * TRADE_STRIDE + ENTRY_PROBE, &[u8::MAX])
        },
        "strategic operative map sentinel inside census",
    );
    let mut fixture = fixture();
    fixture.world.u32(fixture.trade + ENTRY_COUNTRY, 9);
    assert!(matches!(refusal(&fixture), Outcome::Unavailable { .. }));
}
