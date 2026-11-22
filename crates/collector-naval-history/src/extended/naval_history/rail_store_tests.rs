use super::*;
use collector_observers::rail_changes::{Changes, Counts, Target, Watches};
use engine_mutations::railway_mutations;
use engine_native::{cbuilding, memory::SliceMemory, province_definition};
use engine_railways::{
    crailway_manager as manager, crailway_manager::BuildingOutcomes, crailway_manager::RailCarry,
    crailway_manager::fresh_inputs::RailwayBuildingRow,
    crailway_manager::selected_inputs::Levels,
};
use std::cell::RefCell;

const GAME_STATE: usize = 0x400;
const MANAGER: usize = 0x1000;
const RAIL_TABLE: usize = 0x2000;
const PROVINCE_TABLE: usize = 0x3000;
const LIST: usize = 0x3800;
const LIST_CAPACITY: u32 = 64;
const SLOTS: u32 = 200;
const RAILS: [u32; 8] = [1, 2, 3, 5, 7, 70, 130, 199];
const BATCHES: u64 = 4;
const SIZE: usize = 0x8_0000;
const OBJECTS: usize = 0x1_0000;

fn province(id: u32) -> usize {
    OBJECTS + id as usize * 0x200
}
fn definition(id: u32) -> usize {
    0x3_0000 + id as usize * 0x100
}
fn info(id: u32) -> usize {
    0x4_0000 + id as usize * 0x200
}
fn pair_data(id: u32) -> usize {
    info(id) + 0x100
}
fn level_data(id: u32) -> usize {
    info(id) + 0x180
}
fn building(id: u32) -> usize {
    0x6_0000 + id as usize * 0x200
}
fn status(id: u32) -> usize {
    province(id) + cprovince::BUILDING_STATUS
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn word(bytes: &mut [u8], at: usize, value: usize) {
    put(bytes, at, &(value as u64).to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    word(bytes, at, data);
    put(bytes, at + 8, &count.to_le_bytes());
    put(bytes, at + 12, &count.to_le_bytes());
}

fn set_pairs(bytes: &mut [u8], id: u32, pairs: &[(i32, i32)]) {
    header(
        bytes,
        info(id) + cprovince_railway_info::NEIGHBOR_PAIRS,
        pair_data(id),
        pairs.len() as u32,
    );
    for (index, (neighbour, level)) in pairs.iter().enumerate() {
        put(bytes, pair_data(id) + index * 8, &neighbour.to_le_bytes());
        put(bytes, pair_data(id) + index * 8 + 4, &level.to_le_bytes());
    }
}

fn cooldown(bytes: &[u8], id: u32) -> i32 {
    let at = info(id) + cprovince_railway_info::COOLDOWN_TICKS;
    i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn set_cooldown(bytes: &mut [u8], id: u32, ticks: i32) {
    put(
        bytes,
        info(id) + cprovince_railway_info::COOLDOWN_TICKS,
        &ticks.to_le_bytes(),
    );
}

fn list(bytes: &[u8]) -> Vec<u32> {
    let at = MANAGER + manager::COOLDOWN_PROVINCES + 12;
    let count = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    (0..count)
        .map(|index| {
            u32::from_le_bytes(
                bytes[LIST + index * 4..LIST + index * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        })
        .collect()
}

fn set_list(bytes: &mut [u8], ids: &[u32]) {
    let at = MANAGER + manager::COOLDOWN_PROVINCES;
    word(bytes, at, LIST);
    put(bytes, at + 8, &LIST_CAPACITY.to_le_bytes());
    put(bytes, at + 12, &(ids.len() as u32).to_le_bytes());
    for (index, id) in ids.iter().enumerate() {
        put(bytes, LIST + index * 4, &id.to_le_bytes());
    }
}

fn set_on_cooldown(bytes: &mut [u8], id: u32, ticks: i32) {
    if cooldown(bytes, id) < 1 {
        let mut ids = list(bytes);
        ids.push(id);
        set_list(bytes, &ids);
    }
    set_cooldown(bytes, id, ticks);
}

fn countdown(bytes: &mut [u8]) {
    let mut ids = list(bytes);
    for index in (0..ids.len()).rev() {
        let id = ids[index];
        let value = cooldown(bytes, id) - 1;
        set_cooldown(bytes, id, value);
        if value < 1 {
            ids.swap_remove(index);
        }
    }
    set_list(bytes, &ids);
}

fn set_health(bytes: &mut [u8], id: u32, level: i16, healthy: i16, partial: i64) {
    put(bytes, building(id) + cbuilding::LEVEL, &level.to_le_bytes());
    put(
        bytes,
        building(id) + cbuilding::HEALTHY_LEVEL,
        &healthy.to_le_bytes(),
    );
    put(
        bytes,
        building(id) + cbuilding::PARTIAL_HEALTH,
        &partial.to_le_bytes(),
    );
}

fn add_info(bytes: &mut [u8], id: u32) {
    word(bytes, RAIL_TABLE + id as usize * 8, info(id));
    word(bytes, info(id), cprovince_railway_info::VTABLE_RVA.0);
    word(
        bytes,
        info(id) + cprovince_railway_info::PROVINCE_RECORD,
        definition(id),
    );
    word(
        bytes,
        info(id) + cprovince_railway_info::BUILDING_STATUS,
        status(id),
    );
    header(
        bytes,
        info(id) + cprovince_railway_info::LEVELS_BY_NEIGHBOR_INDEX,
        level_data(id),
        3,
    );
    for (index, level) in [id as i32, 0, -(id as i32)].into_iter().enumerate() {
        put(bytes, level_data(id) + index * 4, &level.to_le_bytes());
    }
    if id != 5 {
        word(
            bytes,
            info(id) + cprovince_railway_info::BUILDING,
            building(id),
        );
        word(bytes, building(id), cbuilding::VTABLE_RVA.0);
        word(bytes, building(id) + cbuilding::BUILDING_STATUS, status(id));
    }
    let mut pairs = Vec::new();
    if id > 1 {
        pairs.push((id as i32 - 1, 2));
    }
    if id + 1 < SLOTS {
        pairs.push((id as i32 + 1, 3));
    }
    set_pairs(bytes, id, &pairs);
}

fn world() -> Vec<u8> {
    let mut bytes = vec![0; SIZE];
    word(
        &mut bytes,
        GAME_STATE + engine_native::cgame_state::RAIL_MANAGER,
        MANAGER,
    );
    header(
        &mut bytes,
        GAME_STATE + engine_native::cgame_state::PROVINCES,
        PROVINCE_TABLE,
        SLOTS,
    );
    word(&mut bytes, MANAGER, manager::VTABLE_RVA.0);
    header(
        &mut bytes,
        MANAGER + manager::PROVINCE_INFO_TABLE,
        RAIL_TABLE,
        SLOTS,
    );
    for id in 1..SLOTS {
        word(&mut bytes, PROVINCE_TABLE + id as usize * 8, province(id));
        word(&mut bytes, province(id), cprovince::VTABLE_RVA.0);
        put(&mut bytes, province(id) + cprovince::ID, &id.to_le_bytes());
        word(
            &mut bytes,
            province(id) + cprovince::DEFINITION,
            definition(id),
        );
        put(
            &mut bytes,
            definition(id) + province_definition::ID,
            &id.to_le_bytes(),
        );
        put(
            &mut bytes,
            definition(id) + province_definition::NEIGHBOUR_COUNT,
            &3_i32.to_le_bytes(),
        );
    }
    for id in RAILS {
        add_info(&mut bytes, id);
    }
    set_list(&mut bytes, &[]);
    set_on_cooldown(&mut bytes, 2, 5);
    set_health(&mut bytes, 1, 2, 2, 0);
    set_health(&mut bytes, 2, 1, 1, 0);
    set_health(&mut bytes, 3, 3, 1, 5_000_000);
    bytes
}

struct Calls<'a> {
    bytes: &'a [u8],
    calls: RefCell<Vec<usize>>,
}
impl<'a> Calls<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            calls: RefCell::default(),
        }
    }
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

fn slot_reads(reads: &[usize]) -> usize {
    let slots = SLOTS as usize * 8;
    reads
        .iter()
        .filter(|&&at| {
            (RAIL_TABLE..RAIL_TABLE + slots).contains(&at)
                || (PROVINCE_TABLE..PROVINCE_TABLE + slots).contains(&at)
                || at >= OBJECTS
        })
        .count()
}

fn batches_read(reads: &[usize]) -> Vec<u64> {
    let mut batches: Vec<u64> = reads
        .iter()
        .filter(|&&at| (RAIL_TABLE..RAIL_TABLE + SLOTS as usize * 8).contains(&at))
        .map(|&at| ((at - RAIL_TABLE) / 8 / manager::PROVINCE_BATCH) as u64)
        .collect();
    batches.sort_unstable();
    batches.dedup();
    batches
}

#[derive(Debug, PartialEq)]
struct Rows {
    cooldowns: Vec<cprovince_railway_info::Row>,
    edges: Vec<rail_neighbor::Row>,
    buildings: Vec<RailwayBuildingRow>,
    levels: Vec<i32>,
    level_required: usize,
    outcomes: BuildingOutcomes,
}

impl Rows {
    fn cooldown(&self, province: u32) -> i32 {
        self.cooldowns
            .iter()
            .find(|row| row.province == province)
            .unwrap()
            .cooldown_ticks
    }
}

struct Hour {
    rows: Rows,
    counts: Counts,
    reads: Vec<usize>,
}

fn run(memory: &Calls<'_>, carry: &mut impl RailCarry) -> Rows {
    let (mut cooldowns, mut edges, mut buildings, mut values) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut levels = Levels {
        values: &mut values,
        len: 0,
        required: 0,
    };
    let outcomes = manager::walk_with_selection_carried(
        memory,
        0,
        GAME_STATE,
        &mut cooldowns,
        &mut edges,
        &mut buildings,
        &mut levels,
        carry,
    );
    let (level_len, level_required) = (levels.len, levels.required);
    Rows {
        cooldowns: cooldowns[..outcomes.rows.provinces.len()].to_vec(),
        edges: edges[..outcomes.rows.edges.len()].to_vec(),
        buildings: buildings[..outcomes.buildings.len()].to_vec(),
        levels: values[..level_len].to_vec(),
        level_required,
        outcomes,
    }
}

fn fresh(bytes: &[u8]) -> Rows {
    run(&Calls::new(bytes), &mut manager::Fresh)
}

fn walk(bytes: &[u8], store: &mut RailStore, changes: &dyn Changes) -> Hour {
    let memory = Calls::new(bytes);
    let mut reuse = RailReuse::begin(store, changes, |list| {
        manager::cooldown_list(&memory, 0, GAME_STATE, list)
    });
    let rows = run(&memory, &mut reuse);
    let complete = [
        rows.outcomes.rows.provinces,
        rows.outcomes.rows.edges,
        rows.outcomes.buildings,
    ]
    .iter()
    .all(|outcome| matches!(outcome, Outcome::Rows { .. }));
    reuse.finish(complete);
    Hour {
        rows,
        counts: store.counts(),
        reads: memory.calls.into_inner(),
    }
}

fn site(name: &str) -> usize {
    railway_mutations::MUTATIONS
        .iter()
        .position(|(site, _)| site.name == name)
        .unwrap()
}

fn split(counts: Counts) -> (u64, u64) {
    (counts.reused, counts.read)
}

fn domain() -> Watches {
    Watches::enabled(256)
}

#[test]
fn a_quiet_hour_reads_no_slot_and_publishes_a_fresh_walks_rows() {
    let bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    let expected = fresh(&bytes);
    assert_eq!(
        expected.outcomes.buildings,
        Outcome::Rows { len: RAILS.len() }
    );
    assert_eq!(expected.outcomes.rows.edges, Outcome::Rows { len: 14 });
    assert!(
        expected.buildings[..4]
            .iter()
            .all(|row| row.building.is_ok() && row.selection.unwrap().building_level.is_ok())
    );

    let first = walk(&bytes, &mut store, &rails);
    assert_eq!(first.rows, expected);
    assert_eq!(split(first.counts), (0, RAILS.len() as u64));
    assert_eq!(first.counts.read_batches, BATCHES);
    assert!(first.counts.full);
    assert!(
        slot_reads(&first.reads) > 0,
        "control: a full walk reads slots"
    );

    for _ in 0..3 {
        rails.boundary(false);
        let hour = walk(&bytes, &mut store, &rails);
        assert_eq!(hour.rows, expected);
        assert_eq!(slot_reads(&hour.reads), 0, "a quiet hour reads no slot");
        assert_eq!(split(hour.counts), (RAILS.len() as u64, 0));
        assert_eq!(
            (hour.counts.carried_batches, hour.counts.read_batches),
            (BATCHES, 0)
        );
        assert_eq!(hour.counts.reused_edges, expected.edges.len() as u64);
        assert!(!hour.counts.full);
    }
    assert!(store.allocated_bytes() > 0);
}

#[test]
fn a_hooked_write_reads_only_its_batch() {
    let mut bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    let hour = |bytes: &[u8], store: &mut RailStore| {
        rails.boundary(false);
        let hour = walk(bytes, store, &rails);
        assert_eq!(hour.rows, fresh(bytes));
        hour
    };

    drop(rails.notify(site("set_province_on_cooldown"), Target::Province(Some(70))));
    set_on_cooldown(&mut bytes, 70, 72);
    let after = hour(&bytes, &mut store);
    assert_eq!(batches_read(&after.reads), vec![1]);
    assert_eq!(split(after.counts), (7, 1));
    assert_eq!(after.rows.cooldown(70), 72);

    drop(rails.notify(site("set_railway"), Target::Province(Some(130))));
    set_pairs(&mut bytes, 130, &[(129, 2), (131, 6)]);
    put(&mut bytes, level_data(130) + 4, &6_i32.to_le_bytes());
    let after = hour(&bytes, &mut store);
    assert_eq!(batches_read(&after.reads), vec![2]);
    assert!(after.rows.edges.contains(&rail_neighbor::Row {
        from_province: 130,
        to_province: 131,
        level: 6,
    }));

    drop(rails.mark_building(engine_native::cbuilding_status::PROVINCE_FLAG, 3));
    set_health(&mut bytes, 3, 3, 2, 7_000_000);
    let after = hour(&bytes, &mut store);
    assert_eq!(batches_read(&after.reads), vec![0]);
    assert_eq!(split(after.counts), (7, 1));

    drop(rails.notify(site("construct_info"), Target::Province(Some(100))));
    add_info(&mut bytes, 100);
    let after = hour(&bytes, &mut store);
    assert_eq!(batches_read(&after.reads), vec![1]);
    assert_eq!(split(after.counts), (8, 1));
    assert_eq!(after.rows.outcomes.buildings, Outcome::Rows { len: 9 });

    drop(rails.notify(site("destroy_info"), Target::Province(Some(100))));
    put(&mut bytes, RAIL_TABLE + 100 * 8, &0_u64.to_le_bytes());
    let after = hour(&bytes, &mut store);
    assert_eq!(batches_read(&after.reads), vec![1]);
    assert_eq!(split(after.counts), (8, 0));

    let quiet = hour(&bytes, &mut store);
    assert_eq!(slot_reads(&quiet.reads), 0);
    assert_eq!(quiet.counts.carried_batches, BATCHES);
    assert!(!rails.poisoned());
}

#[test]
fn the_daily_countdown_reads_every_listed_province_including_duplicates() {
    let mut bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    for (province, ticks) in [(7, 0), (7, 0), (130, 3)] {
        drop(rails.notify(
            site("set_province_on_cooldown"),
            Target::Province(Some(province as usize)),
        ));
        set_on_cooldown(&mut bytes, province, ticks);
    }
    assert_eq!(list(&bytes), vec![2, 7, 7, 130]);
    walk(&bytes, &mut store, &rails);
    let quiet = walk(&bytes, &mut store, &rails);
    assert_eq!(quiet.counts.carried_batches, BATCHES);

    drop(rails.notify(site("update_cooldowns"), Target::Countdown));
    countdown(&mut bytes);
    assert_eq!(list(&bytes), vec![2, 130]);
    assert_eq!(cooldown(&bytes, 7), -2);
    let day = walk(&bytes, &mut store, &rails);
    assert_eq!(day.rows, fresh(&bytes));
    assert!(day.counts.countdown);
    assert_eq!(batches_read(&day.reads), vec![0, 2]);
    assert_eq!(
        [2, 7, 130].map(|province| day.rows.cooldown(province)),
        [4, -2, 2]
    );

    drop(rails.notify(site("update_cooldowns"), Target::Countdown));
    countdown(&mut bytes);
    let day = walk(&bytes, &mut store, &rails);
    assert_eq!(day.rows, fresh(&bytes));
    assert_eq!(batches_read(&day.reads), vec![0, 2]);
    assert_eq!(
        [2, 7, 130].map(|province| day.rows.cooldown(province)),
        [3, -2, 1]
    );
    let quiet = walk(&bytes, &mut store, &rails);
    assert_eq!(slot_reads(&quiet.reads), 0);
    assert!(!rails.poisoned());
}

#[test]
fn an_unobserved_cooldown_list_entry_reads_every_batch() {
    let mut bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    set_list(&mut bytes, &[2, 2]);
    let caught = walk(&bytes, &mut store, &rails);
    assert_eq!(caught.rows, fresh(&bytes));
    assert_eq!(caught.counts.read_batches, BATCHES);
    let after = walk(&bytes, &mut store, &rails);
    assert_eq!(after.rows, fresh(&bytes));
    assert_eq!(after.counts.carried_batches, BATCHES);
}

#[test]
fn a_write_naming_no_info_an_invalidation_or_a_lost_forwarder_reads_every_batch() {
    let bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    let full_then_quiet = |store: &mut RailStore, what: &str| {
        let hour = walk(&bytes, store, &rails);
        assert_eq!(
            (hour.counts.carried_batches, hour.counts.read_batches),
            (0, BATCHES),
            "{what}"
        );
        assert_eq!(hour.rows, fresh(&bytes), "{what}");
        let quiet = walk(&bytes, store, &rails);
        assert_eq!(quiet.counts.carried_batches, BATCHES, "{what}");
    };
    for name in [
        "read_info",
        "construct_manager",
        "destroy_manager",
        "read_manager",
    ] {
        drop(rails.notify(site(name), Target::Every));
        full_then_quiet(&mut store, name);
    }
    rails.boundary(true);
    full_then_quiet(&mut store, "invalidation");
    rails.set_forwarder(false);
    let unobserved = walk(&bytes, &mut store, &rails);
    assert_eq!(unobserved.counts.read_batches, BATCHES);
    rails.set_forwarder(true);
    full_then_quiet(&mut store, "forwarder");
    assert!(!rails.poisoned());
}

#[test]
fn an_unmarked_change_in_a_read_batch_is_caught() {
    let mut bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    drop(rails.notify(site("set_railway"), Target::Province(Some(1))));
    set_pairs(&mut bytes, 7, &[(6, 2), (8, 3), (9, 1)]);
    let hour = walk(&bytes, &mut store, &rails);
    assert_eq!(hour.rows, fresh(&bytes));
    assert_eq!(hour.counts.read_batches, BATCHES);

    let mut bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    put(&mut bytes, RAIL_TABLE + 199 * 8, &0_u64.to_le_bytes());
    drop(rails.notify(
        site("set_province_on_cooldown"),
        Target::Province(Some(198)),
    ));
    let hour = walk(&bytes, &mut store, &rails);
    assert_eq!(hour.rows, fresh(&bytes));
}

#[test]
fn a_quiet_batch_needs_its_held_read_whole_and_an_incomplete_walk_holds_nothing() {
    let mut bytes = world();
    word(
        &mut bytes,
        info(3) + cprovince_railway_info::BUILDING,
        SIZE + 0x100,
    );
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    for _ in 0..2 {
        let hour = walk(&bytes, &mut store, &rails);
        assert_eq!(batches_read(&hour.reads), vec![0]);
        assert_eq!(split(hour.counts), (7, 1));
        assert!(hour.rows.buildings[2].building.is_err());
        assert_eq!(hour.rows, fresh(&bytes));
    }
    assert!(!rails.poisoned());

    let mut bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    header(
        &mut bytes,
        info(2) + cprovince_railway_info::NEIGHBOR_PAIRS,
        SIZE + 0x100,
        2,
    );
    drop(rails.notify(site("set_railway"), Target::Province(Some(2))));
    let failed = walk(&bytes, &mut store, &rails);
    assert!(matches!(
        failed.rows.outcomes.rows.edges,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(failed.rows, fresh(&bytes));
    let restored = world();
    let after = walk(&restored, &mut store, &rails);
    assert_eq!(after.counts.read_batches, BATCHES);
    assert_eq!(after.rows, fresh(&restored));
}

#[test]
fn a_forgotten_store_reads_every_batch() {
    let bytes = world();
    let rails = domain();
    let mut store = RailStore::new();
    walk(&bytes, &mut store, &rails);
    store.forget();
    let hour = walk(&bytes, &mut store, &rails);
    assert_eq!(hour.counts.read_batches, BATCHES);
    let hour = walk(&bytes, &mut store, &rails);
    assert_eq!(hour.counts.carried_batches, BATCHES);
}
