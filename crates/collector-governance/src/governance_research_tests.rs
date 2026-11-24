
use super::*;
use crate::country_input_changes::{Domain, Table};
use engine_governance_tests::{IMAGE_LEN, IMAGE_START};
use engine_native::cgame_state;
use engine_native_parts::modifier_definition;
use engine_research::{climited_use_tech_bonus as bonus, research_inputs, research_modifiers};

const BASE: usize = 0;
const HEAP_START: usize = 0x1000;
const HEAP_END: usize = 0x3_0000;
const GAME_STATE: usize = 0x1000;
const COUNTRY_TABLE: usize = 0x2000;
const LINK_TABLE: usize = 0x2100;
const METADATA: usize = 0x3000;

const COST_SCALE: i64 = 130_000;
const AHEAD_PENALTY: i64 = 175_000;
const REASON: &str = "governance acquisition panicked";

struct Tech {
    handle: u32,
    database_index: i32,
    token: &'static [u8],
    year: i32,
    progress: i64,
    cost: i64,
    applied_speed: i64,
    category_speed: i64,
    experience: Option<i64>,
    bonus_ids: &'static [i32],
}

struct Slot {
    index: u32,
    speed: i64,
    tech: Option<Tech>,
}

struct Bonus {
    id: i32,
    name: &'static [u8],
    speed: i64,
    ahead: i64,
}

struct CountrySpec {
    census_slot: u32,
    id: u32,
    origin: usize,
    slots: &'static [Slot],
    bonuses: &'static [Bonus],
    country_speed: i64,
    source: Option<(&'static [u8], i64)>,
}

const STATUS: usize = 0x1000;
const SLOTS: usize = 0x1200;
const SLOT: usize = 0x1300;
const TECH: usize = 0x2000;
const TEMPLATE: usize = 0x3000;
const IDS: usize = 0x4000;
const BONUS_ROSTER: usize = 0x4200;
const BONUS: usize = 0x4400;
const STORAGE: usize = 0x5000;
const CHILDREN: usize = 0x5100;
const WRAPPER: usize = 0x5200;
const CHILD_STORAGE: usize = 0x5400;

const FIRST: CountrySpec = CountrySpec {
    census_slot: 1,
    id: 1,
    origin: 0x1_0000,
    slots: &[
        Slot {
            index: 2,
            speed: 110_000,
            tech: Some(Tech {
                handle: 21,
                database_index: 4,
                token: b"electronic_eng",
                year: 1937,
                progress: 125_000,
                cost: 1_500_000,
                applied_speed: 0,
                category_speed: 5_000,
                experience: None,
                bonus_ids: &[],
            }),
        },
        Slot {
            index: 1,
            speed: 100_000,
            tech: None,
        },
        Slot {
            index: 0,
            speed: 150_000,
            tech: Some(Tech {
                handle: 7,
                database_index: 1,
                token: b"radio",
                year: 1936,
                progress: 500_000,
                cost: 2_000_000,
                applied_speed: 100_000,
                category_speed: -25_001,
                experience: Some(100_003),
                bonus_ids: &[-7],
            }),
        },
    ],
    bonuses: &[Bonus {
        id: -7,
        name: b"bonus_radio",
        speed: 100_000,
        ahead: 200_000,
    }],
    country_speed: 15_000,
    source: None,
};

const SECOND: CountrySpec = CountrySpec {
    census_slot: 3,
    id: 3,
    origin: 0x2_0000,
    slots: &[Slot {
        index: 0,
        speed: 90_000,
        tech: Some(Tech {
            handle: 9,
            database_index: 2,
            token: b"infantry_weapon",
            year: 1938,
            progress: 250_000,
            cost: 3_000_000,
            applied_speed: 50_000,
            category_speed: 12_345,
            experience: None,
            bonus_ids: &[-9],
        }),
    }],
    bonuses: &[Bonus {
        id: -9,
        name: b"bonus_infantry",
        speed: 50_000,
        ahead: 0,
    }],
    country_speed: -12_000,
    source: Some((b"slow_idea", -12_000)),
};

struct World {
    heap: Vec<u8>,
    image: Vec<u8>,
}

fn window(bytes: &[u8], start: usize, address: usize, len: usize) -> Option<&[u8]> {
    let offset = address.checked_sub(start)?;
    bytes.get(offset..offset.checked_add(len)?)
}

impl engine_native_memory::Memory for World {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        let source = window(&self.image, IMAGE_START, address, out.len())
            .or_else(|| window(&self.heap, HEAP_START, address, out.len()));
        match source {
            Some(bytes) => {
                out.copy_from_slice(bytes);
                true
            }
            None => false,
        }
    }
}

impl World {
    fn new() -> Self {
        let mut world = Self {
            heap: vec![0; HEAP_END - HEAP_START],
            image: vec![0; IMAGE_LEN],
        };
        world.array(GAME_STATE + cgame_state::COUNTRIES, COUNTRY_TABLE, 4);
        world.array(GAME_STATE + cgame_state::COUNTRY_LINKS, LINK_TABLE, 4);
        for spec in [&FIRST, &SECOND] {
            world.put(
                COUNTRY_TABLE + spec.census_slot as usize * 8,
                (spec.origin as u64).to_le_bytes(),
            );
            world.put(
                LINK_TABLE + spec.id as usize * 4,
                spec.census_slot.to_le_bytes(),
            );
            world.country(spec);
        }
        world.image_put(ctechnology::BASE_COST_SCALE_RVA, &COST_SCALE.to_le_bytes());
        world.image_put(ctechnology::BASE_COST_DEBUG_FLAG_RVA, &[0]);
        world.image_put(
            ctechnology::BASE_YEAR_AHEAD_PENALTY_FACTOR_RVA,
            &AHEAD_PENALTY.to_le_bytes(),
        );
        world.image_put(
            modifier_definition::TABLE_RVA,
            &(METADATA as u64).to_le_bytes(),
        );
        assert!(
            research_inputs::Context::read(&world, BASE).is_ok(),
            "research cost defines must be readable"
        );
        assert!(
            research_modifiers::Context::read(&world, BASE).is_ok(),
            "the module data window must cover the research modifier defines"
        );
        world
    }

    fn put<const L: usize>(&mut self, at: usize, bytes: [u8; L]) {
        self.heap[at - HEAP_START..at - HEAP_START + L].copy_from_slice(&bytes);
    }

    fn image_put(&mut self, rva: engine_sites::Rva, bytes: &[u8]) {
        let at = BASE + rva.0 - IMAGE_START;
        self.image[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn pointer(&mut self, at: usize, pointer: usize) {
        self.put(at, (pointer as u64).to_le_bytes());
    }

    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.pointer(at, data);
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }

    fn text(&mut self, at: usize, text: &[u8]) {
        assert!(text.len() <= 15);
        self.heap[at - HEAP_START..at - HEAP_START + text.len()].copy_from_slice(text);
        self.put(at + 16, (text.len() as u64).to_le_bytes());
        self.put(at + 24, 15_u64.to_le_bytes());
    }

    fn modifier(&mut self, accumulator: usize, storage: usize, key: u32, raw: i64) {
        self.array(accumulator, storage, 1);
        self.put(storage, key.to_le_bytes());
        self.put(storage + 8, raw.to_le_bytes());
    }

    fn country(&mut self, spec: &CountrySpec) {
        let o = spec.origin;
        let status = o + STATUS;
        self.put(
            o,
            (BASE as u64 + ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        self.put(o + ccountry::ID, spec.id.to_le_bytes());
        self.put(o + ccountry::OWNED_STATE_COUNT, 1_i32.to_le_bytes());
        self.pointer(o + ccountry::TECHNOLOGY_STATUS, status);
        self.put(
            status,
            (BASE as u64 + ctechnology_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        self.pointer(status + ctechnology_status::COUNTRY, o);
        self.array(
            status + ctechnology_status::RESEARCH_SLOTS_DATA,
            o + SLOTS,
            spec.slots.len() as u32,
        );
        for (i, slot) in spec.slots.iter().enumerate() {
            let at = o + SLOT + i * 0x100;
            self.pointer(o + SLOTS + i * 8, at);
            self.put(
                at,
                (BASE as u64 + cresearch_slot::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.pointer(at + cresearch_slot::STATUS, status);
            self.put(
                at + cresearch_slot::RESEARCH_SPEED,
                slot.speed.to_le_bytes(),
            );
            self.put(at + cresearch_slot::INDEX, slot.index.to_le_bytes());
            let Some(tech) = &slot.tech else {
                self.put(
                    at + cresearch_slot::TECHNOLOGY_HANDLE,
                    cresearch_slot::IDLE_HANDLE.to_le_bytes(),
                );
                continue;
            };
            let technology = o + TECH + i * 0x400;
            let template = o + TEMPLATE + i * 0x800;
            let ids = o + IDS + i * 0x40;
            self.put(
                at + cresearch_slot::TECHNOLOGY_HANDLE,
                tech.handle.to_le_bytes(),
            );
            self.pointer(at + cresearch_slot::TECHNOLOGY, technology);
            self.put(
                technology,
                (BASE as u64 + ctechnology::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.put(
                technology + ctechnology::DEFINITION_INDEX,
                tech.handle.to_le_bytes(),
            );
            self.pointer(technology + ctechnology::STATUS, status);
            self.pointer(technology + ctechnology::TEMPLATE, template);
            self.put(
                technology + ctechnology::CLASSIFIER,
                ctechnology::CLASSIFIER_ACTIVE.to_le_bytes(),
            );
            self.put(
                technology + ctechnology::LIVE_PROGRESS,
                tech.progress.to_le_bytes(),
            );
            self.put(
                technology + ctechnology::APPLIED_SPEED_BONUS,
                tech.applied_speed.to_le_bytes(),
            );
            self.array(
                technology + ctechnology::APPLIED_BONUS_IDS,
                ids,
                tech.bonus_ids.len() as u32,
            );
            for (k, id) in tech.bonus_ids.iter().enumerate() {
                self.put(ids + k * 4, id.to_le_bytes());
            }
            self.put(technology + 0x190, tech.category_speed.to_le_bytes());
            if let Some(experience) = tech.experience {
                self.put(technology + 0x1e8, [1]);
                self.put(template + 0x538, experience.to_le_bytes());
            }
            self.put(
                template + ctechnology_template::DATABASE_INDEX,
                tech.database_index.to_le_bytes(),
            );
            self.put(
                template + ctechnology_template::TECHNOLOGY_HANDLE,
                tech.handle.to_le_bytes(),
            );
            self.put(
                template + ctechnology_template::YEAR,
                tech.year.to_le_bytes(),
            );
            self.put(
                template + ctechnology_template::MAX_LEVEL,
                1_i32.to_le_bytes(),
            );
            self.put(
                template + ctechnology_template::RESEARCH_COST,
                tech.cost.to_le_bytes(),
            );
            self.text(template + ctechnology_template::ID, tech.token);
        }
        self.array(
            status + ctechnology_status::LIMITED_BONUSES,
            o + BONUS_ROSTER,
            spec.bonuses.len() as u32,
        );
        for (j, value) in spec.bonuses.iter().enumerate() {
            let at = o + BONUS + j * 0x100;
            self.pointer(o + BONUS_ROSTER + j * 8, at);
            self.put(at, (BASE as u64 + bonus::VTABLE_RVA.0 as u64).to_le_bytes());
            self.put(at + bonus::ID, value.id.to_le_bytes());
            self.text(at + bonus::NAME, value.name);
            self.put(at + bonus::SPEED_BONUS, value.speed.to_le_bytes());
            self.put(at + bonus::AHEAD_REDUCTION, value.ahead.to_le_bytes());
        }
        let accumulator = o + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
        self.modifier(accumulator, o + STORAGE, 0xcd, spec.country_speed);
        if let Some((name, raw)) = spec.source {
            self.array(accumulator + 0x18, o + CHILDREN, 1);
            self.pointer(o + CHILDREN, o + WRAPPER);
            let child = o + WRAPPER + 0x10;
            self.modifier(child, o + CHILD_STORAGE, 0xcd, raw);
            self.text(child + 0x48, name);
        }
    }
}

fn context(hour: i32, force_full: bool) -> CaptureContext {
    CaptureContext {
        base: BASE,
        game_state: GAME_STATE,
        hour,
        force_full,
    }
}

fn captured() -> Captured {
    Captured::with_capacities(&Capacities::default())
}

#[derive(Debug, PartialEq)]
struct Research {
    inputs: Vec<research_inputs::Row>,
    bonuses: Vec<research_inputs::BonusRow>,
    modifiers: Vec<research_modifiers::Row>,
}

fn reference(world: &World) -> Research {
    let countries = ccountry::Reader::new(world, BASE, GAME_STATE).expect("census readable");
    let input_context = research_inputs::Context::read(world, BASE);
    let modifier_context = research_modifiers::Context::read(world, BASE);
    let mut result = Research {
        inputs: vec![],
        bonuses: vec![],
        modifiers: vec![],
    };
    let mut visited = vec![];
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot).expect("census slot readable") else {
            continue;
        };
        visited.push((country.canonical_slot, country.id, country.pointer));
        let mut inputs = vec![research_inputs::Row::default(); FIRST.slots.len()];
        let mut bonuses = vec![research_inputs::BonusRow::default(); FIRST.bonuses.len()];
        let mut modifiers = Vec::new();
        let (input_outcome, bonus_outcome) = research_inputs::walk_country(
            world,
            BASE,
            country.pointer,
            country.id,
            input_context,
            &mut inputs,
            &mut bonuses,
        );
        let Outcome::Rows { len: input_len } = input_outcome else {
            panic!("country {} inputs: {input_outcome:?}", country.id);
        };
        let Outcome::Rows { len: bonus_len } = bonus_outcome else {
            panic!("country {} bonuses: {bonus_outcome:?}", country.id);
        };
        let modifier_outcome = research_modifiers::grants::walk_country_with_applied(
            world,
            BASE,
            &countries,
            country,
            modifier_context,
            &inputs[..input_len],
            0,
            &bonuses[..bonus_len],
            engine_native_memory::walk::Buffer::Growing(&mut modifiers),
        );
        let Outcome::Rows { len: modifier_len } = modifier_outcome else {
            panic!("country {} modifiers: {modifier_outcome:?}", country.id);
        };
        assert!(input_len > 0 && bonus_len > 0 && modifier_len > 0);
        result.inputs.extend_from_slice(&inputs[..input_len]);
        result.bonuses.extend_from_slice(&bonuses[..bonus_len]);
        result
            .modifiers
            .extend_from_slice(&modifiers[..modifier_len]);
    }
    assert_eq!(
        visited,
        [
            (FIRST.census_slot, FIRST.id, FIRST.origin),
            (SECOND.census_slot, SECOND.id, SECOND.origin),
        ],
        "census order skips the null slot"
    );
    result
}

fn research(captured: &Captured) -> Research {
    fn complete<R: Clone>(rows: &CaptureRows<R>, table: &str) -> Vec<R> {
        let Outcome::Rows { len } = rows.outcome else {
            panic!("{table} not complete: {:?}", rows.outcome);
        };
        rows.rows[..len].to_vec()
    }
    Research {
        inputs: complete(&captured.research_inputs, "ResearchInputs"),
        bonuses: complete(&captured.research_bonuses, "ResearchAppliedBonuses"),
        modifiers: complete(&captured.research_modifiers, "ResearchModifiers"),
    }
}

fn research_tables(captured: &Captured) -> Vec<TableView> {
    let tables = captured.tables();
    RESEARCH
        .iter()
        .map(|&index| tables[index].clone())
        .collect()
}

fn loaned_only(world: &World) -> Captured {
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture(world, context(0, true), &mut SliceFingerprints::new());
    captured.restore_research(part);
    captured
}

#[test]
fn gap_research_grants_research_part_rows_match_independent_per_country_walks() {
    let world = World::new();
    let expected = reference(&world);
    let captured = loaned_only(&world);
    let actual = research(&captured);
    assert_eq!(actual.inputs.len(), 4);
    assert_eq!(actual.bonuses.len(), 2);
    assert_eq!(actual.modifiers.len(), 41);
    assert_eq!(
        actual
            .modifiers
            .iter()
            .filter(|row| row.kind == research_modifiers::Kind::GrantCounter)
            .count(),
        2
    );
    assert_eq!(
        actual
            .modifiers
            .iter()
            .filter(|row| row.kind == research_modifiers::Kind::GrantUses)
            .count(),
        2
    );
    for country in [FIRST.id, SECOND.id] {
        assert!(actual.inputs.iter().any(|r| r.slot.country == country));
        assert!(actual.bonuses.iter().any(|r| r.country == country));
        assert!(actual.modifiers.iter().any(|r| r.country == country));
    }
    assert_eq!(actual.inputs, expected.inputs);
    assert_eq!(actual.bonuses, expected.bonuses);
    assert_eq!(actual.modifiers, expected.modifiers);
    for view in research_tables(&captured) {
        assert!(matches!(view, TableView::Complete(_)), "{view:?}");
    }
}

#[test]
fn gap_research_grants_uncertified_hourly_fallback_does_not_read_inventory_counters() {
    struct Uncertified<'a> {
        world: &'a World,
        counters: std::cell::Cell<usize>,
    }
    impl Memory for Uncertified<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if [FIRST.origin, SECOND.origin].iter().any(|origin| {
                address == origin + STATUS + engine_research::ctechnology_status::NEXT_BONUS_ID
            }) {
                self.counters.set(self.counters.get() + 1);
                return false;
            }
            self.world.copy(address, destination)
        }
    }
    let world = World::new();
    let memory = Uncertified {
        world: &world,
        counters: Default::default(),
    };
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture_changed(
        &memory,
        context(1, false),
        &mut SliceFingerprints::new(),
        &Domain::new(),
    );
    assert!(!part.changed.certified);
    captured.restore_research(part);
    let actual = research(&captured);
    assert_eq!(memory.counters.get(), 0);
    assert!(
        actual
            .modifiers
            .iter()
            .all(|row| row.index != research_modifiers::grants::INVENTORY_INDEX)
    );
    assert_eq!(actual.modifiers.len(), 35);
}

#[test]
fn seed_then_bitmap_installation_keeps_grant_inventory_shape_and_certificate() {
    use crate::country_input_changes::Changes;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Installing {
        installed: AtomicBool,
        domain: Domain,
    }
    impl Changes for Installing {
        fn take(&self, table: Table, visit: &mut dyn FnMut(usize)) -> Option<u64> {
            self.installed
                .load(Ordering::Relaxed)
                .then(|| self.domain.take(table, visit))
                .flatten()
        }
    }
    let world = World::new();
    let changes = Installing {
        installed: AtomicBool::new(false),
        domain: Domain::enabled(64),
    };
    let mut fingerprints = SliceFingerprints::new();
    let mut captured = captured();
    let mut part = captured.take_research();
    let mut baseline = None;
    for (hour, forced, installed, certified) in [
        (0, true, false, false),
        (1, false, true, false),
        (2, true, true, true),
    ] {
        changes.installed.store(installed, Ordering::Relaxed);
        part.capture_changed(&world, context(hour, forced), &mut fingerprints, &changes);
        assert_eq!(part.changed.certified, certified, "hour {hour}");
        let rows = part.modifiers.rows[..part.modifiers.outcome.len()].to_vec();
        assert_eq!(
            rows.iter()
                .filter(|r| r.kind == research_modifiers::Kind::GrantCounter)
                .count(),
            2
        );
        if let Some(expected) = &baseline {
            assert_eq!(&rows, expected, "hour {hour}");
        } else {
            baseline = Some(rows);
        }
        assert!(!changes.domain.poisoned(Table::Research));
    }
}

#[test]
fn family_capture_with_loaned_research_yields_the_part_rows() {
    let world = World::new();
    let expected = reference(&world);
    let alone = research_tables(&loaned_only(&world));
    for research_first in [false, true] {
        let mut captured = captured();
        let mut part = captured.take_research();
        if research_first {
            part.capture(&world, context(0, true), &mut SliceFingerprints::new());
            captured.capture(&world, BASE, GAME_STATE, 0, true);
        } else {
            captured.capture(&world, BASE, GAME_STATE, 0, true);
            part.capture(&world, context(0, true), &mut SliceFingerprints::new());
        }
        captured.restore_research(part);
        assert_eq!(
            research(&captured),
            expected,
            "research_first={research_first}"
        );
        assert_eq!(research_tables(&captured), alone);
    }
}

#[test]
fn governance_failure_keeps_successful_research_rows() {
    let world = World::new();
    let expected = reference(&world);
    let alone = research_tables(&loaned_only(&world));
    let context = context(0, true);
    let mut captured = captured();
    let mut part = captured.take_research();
    captured.fail_owned_capture(context, REASON);
    part.capture(&world, context, &mut SliceFingerprints::new());
    captured.restore_research(part);
    assert_eq!(research(&captured), expected);
    assert_eq!(research_tables(&captured), alone);
    let outcomes = captured.outcomes();
    let tables = captured.tables();
    for index in (0..N).filter(|index| !RESEARCH.contains(index)) {
        assert!(replay_data::cadence::should_walk(
            TABLES[index],
            context.hour,
            context.force_full
        ));
        assert_eq!(
            outcomes[index],
            Outcome::Unavailable { reason: REASON },
            "{:?}",
            TABLES[index]
        );
        assert!(matches!(&tables[index], TableView::Unavailable { .. }));
    }
}

#[test]
fn research_part_failure_publishes_reason_or_cadence_skip() {
    let world = World::new();
    let mut captured = captured();
    for force_full in [false, true] {
        for hour in 0..48 {
            let context = context(hour, force_full);
            let mut part = captured.take_research();
            captured.capture(&world, BASE, GAME_STATE, hour, force_full);
            let family = captured.outcomes();
            part.capture(&world, context, &mut SliceFingerprints::new());
            part.fail(context, "research acquisition panicked");
            captured.restore_research(part);
            let outcomes = captured.outcomes();
            let tables = captured.tables();
            for index in RESEARCH {
                let reason = if replay_data::cadence::should_walk(TABLES[index], hour, force_full) {
                    "research acquisition panicked"
                } else {
                    replay_data::cadence::SKIPPED_REASON
                };
                assert_eq!(
                    outcomes[index],
                    Outcome::Unavailable { reason },
                    "{:?} hour {hour} force_full {force_full}",
                    TABLES[index]
                );
                assert!(matches!(&tables[index], TableView::Unavailable { .. }));
            }
            for index in (0..N).filter(|index| !RESEARCH.contains(index)) {
                assert_eq!(outcomes[index], family[index]);
            }
        }
    }
}

fn owners(captured: &Captured) -> Vec<u32> {
    let mut owners: Vec<_> = research(captured)
        .inputs
        .iter()
        .map(|r| r.slot.country)
        .collect();
    owners.dedup();
    owners
}

fn slice_capture(
    world: &impl engine_native_memory::Memory,
    hour: i32,
    fingerprints: &mut SliceFingerprints,
    changes: &Domain,
) -> Captured {
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture_changed(world, context(hour, false), fingerprints, changes);
    captured.restore_research(part);
    captured
}

fn primed(world: &impl engine_native_memory::Memory, changes: &Domain) -> SliceFingerprints {
    let mut fingerprints = SliceFingerprints::new();
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture_changed(world, context(0, true), &mut fingerprints, changes);
    captured.restore_research(part);
    fingerprints
}

#[test]
fn off_full_hours_read_only_the_slice_and_label_rows_with_the_hour() {
    let world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    for (hour, expected) in [(1, vec![1]), (3, vec![3]), (5, vec![]), (25, vec![1])] {
        let captured = slice_capture(&world, hour, &mut fingerprints, &changes);
        assert_eq!(owners(&captured), expected, "hour {hour}");
        let selection = captured.research_selection();
        assert_eq!(selection.hour, hour);
        assert_eq!(selection.early_count, 0);
        assert_eq!(selection.scanned, 2);
        assert_eq!(selection.contains(1), hour % 24 == 1);
        assert_eq!(selection.contains(3), hour % 24 == 3);
        for view in research_tables(&captured) {
            let TableView::Complete(rows) = view else {
                panic!("slice tables complete")
            };
            assert!(
                replay_data::sliced::owners(&rows)
                    .into_iter()
                    .all(|owner| selection.contains(owner))
            );
        }
    }
    let full = research(&loaned_only(&world));
    let slice = research(&slice_capture(&world, 27, &mut fingerprints, &changes));
    let only = |rows: &Research| {
        (
            rows.inputs
                .iter()
                .filter(|r| r.slot.country == 3)
                .cloned()
                .collect::<Vec<_>>(),
            rows.modifiers
                .iter()
                .filter(|r| r.country == 3)
                .cloned()
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(only(&slice), only(&full));
    assert!(!changes.poisoned(Table::Research));
}

#[test]
fn a_marked_country_is_read_within_the_hour_and_an_unmarked_change_is_carried_at_its_slice() {
    let mut world = World::new();
    let changes = Domain::enabled(64);
    let mut fresh = SliceFingerprints::new();
    assert_eq!(
        owners(&slice_capture(&world, 5, &mut fresh, &changes)),
        [1, 3]
    );
    let mut fingerprints = primed(&world, &changes);
    assert!(owners(&slice_capture(&world, 5, &mut fingerprints, &changes)).is_empty());
    let speed = SECOND.origin + SLOT + engine_research::cresearch_slot::RESEARCH_SPEED;
    world.put(speed, 95_000_i64.to_le_bytes());
    changes.write(Table::Research, 3);
    let captured = slice_capture(&world, 6, &mut fingerprints, &changes);
    assert_eq!(owners(&captured), [3]);
    let selection = captured.research_selection();
    assert_eq!(selection.early_count, 1);
    assert!(selection.contains(3) && !selection.contains(1));
    assert!(!research(&captured).modifiers.is_empty());
    assert!(owners(&slice_capture(&world, 7, &mut fingerprints, &changes)).is_empty());
    world.put(
        SECOND.origin + TECH + engine_research::ctechnology::LIVE_PROGRESS,
        260_000_i64.to_le_bytes(),
    );
    assert!(owners(&slice_capture(&world, 8, &mut fingerprints, &changes)).is_empty());
    let captured = slice_capture(&world, 27, &mut fingerprints, &changes);
    assert_eq!(owners(&captured), [3]);
    assert_eq!(research(&captured).inputs[0].progress_raw, 260_000);
    world.put(speed, 97_000_i64.to_le_bytes());
    assert!(owners(&slice_capture(&world, 28, &mut fingerprints, &changes)).is_empty());
    let carried = slice_capture(&world, 51, &mut fingerprints, &changes);
    assert_eq!(owners(&carried), [3]);
    assert_eq!(research(&carried).inputs[0].slot.research_speed_raw, 95_000);
}

#[test]
fn an_uncertified_hour_reads_every_country() {
    let world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    assert!(owners(&slice_capture(&world, 5, &mut fingerprints, &changes)).is_empty());
    changes.write_every(Table::Research);
    assert_eq!(
        owners(&slice_capture(&world, 6, &mut fingerprints, &changes)),
        [1, 3]
    );
    assert!(owners(&slice_capture(&world, 7, &mut fingerprints, &changes)).is_empty());
    changes.boundary(true);
    assert_eq!(
        owners(&slice_capture(&world, 8, &mut fingerprints, &changes)),
        [1, 3]
    );
    assert!(owners(&slice_capture(&world, 9, &mut fingerprints, &changes)).is_empty());
    changes.write(Table::Stability, 3);
    changes.write_every(Table::Stability);
    assert!(owners(&slice_capture(&world, 10, &mut fingerprints, &changes)).is_empty());
    fingerprints.forget();
    assert_eq!(
        owners(&slice_capture(&world, 11, &mut fingerprints, &changes)),
        [1, 3]
    );
    assert!(owners(&slice_capture(&world, 12, &mut fingerprints, &changes)).is_empty());
    assert!(!changes.poisoned(Table::Research));
}

impl World {
}

struct Reads<'a> {
    world: &'a World,
    addresses: std::cell::RefCell<Vec<usize>>,
}

impl engine_native_memory::Memory for Reads<'_> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.addresses.borrow_mut().push(address);
        self.world.copy(address, out)
    }
}

#[test]
fn an_unmarked_slice_reads_only_the_daily_drift() {
    let mut world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    world.put(
        SECOND.origin + TECH + engine_research::ctechnology::LIVE_PROGRESS,
        270_000_i64.to_le_bytes(),
    );
    world.put(
        SECOND.origin + SLOT + engine_research::cresearch_slot::BANKED_DAYS,
        4_200_000_i64.to_le_bytes(),
    );
    let reads = Reads {
        world: &world,
        addresses: Default::default(),
    };
    let carried = slice_capture(&reads, 27, &mut fingerprints, &changes);
    assert_eq!(owners(&carried), [3]);
    let untouched = [
        SECOND.origin + TEMPLATE..SECOND.origin + IDS,
        SECOND.origin + BONUS..SECOND.origin + CHILD_STORAGE + 0x100,
        SECOND.origin + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR
            ..SECOND.origin + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x60,
    ];
    let addresses = reads.addresses.into_inner();
    assert!(
        addresses
            .iter()
            .all(|address| untouched.iter().all(|range| !range.contains(address))),
        "a carried slice reads no template, bonus or modifier"
    );
    let only = |rows: Research| Research {
        inputs: rows
            .inputs
            .into_iter()
            .filter(|row| row.slot.country == SECOND.id)
            .collect(),
        bonuses: rows
            .bonuses
            .into_iter()
            .filter(|row| row.country == SECOND.id)
            .collect(),
        modifiers: rows
            .modifiers
            .into_iter()
            .filter(|row| row.country == SECOND.id)
            .collect(),
    };
    let carried = only(research(&carried));
    assert_eq!(carried, only(research(&loaned_only(&world))));
    assert_eq!(carried.inputs[0].progress_raw, 270_000);
    assert_eq!(carried.inputs[0].slot.banked_days_raw, 4_200_000);
    world.array(
        SECOND.origin + STATUS + ctechnology_status::RESEARCH_SLOTS_DATA,
        SECOND.origin + SLOTS,
        0,
    );
    let read = slice_capture(&world, 51, &mut fingerprints, &changes);
    assert_eq!(only(research(&read)), only(research(&loaned_only(&world))));
}

#[test]
fn a_carried_slice_keeps_each_slots_banked_days_on_its_row() {
    let mut world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    for position in 0..FIRST.slots.len() {
        world.put(
            FIRST.origin + SLOT + position * 0x100 + engine_research::cresearch_slot::BANKED_DAYS,
            (1_000_000 * (position as i64 + 1)).to_le_bytes(),
        );
    }
    let carried = slice_capture(&world, 25, &mut fingerprints, &changes);
    assert_eq!(owners(&carried), [1]);
    let full = research(&loaned_only(&world));
    let first = |rows: Vec<research_inputs::Row>| -> Vec<research_inputs::Row> {
        rows.into_iter()
            .filter(|row| row.slot.country == FIRST.id)
            .collect()
    };
    let carried = first(research(&carried).inputs);
    assert_eq!(carried, first(full.inputs));
    let banked: Vec<i64> = carried.iter().map(|row| row.slot.banked_days_raw).collect();
    assert_eq!(banked, [3_000_000, 2_000_000, 1_000_000]);
}

struct Faulty<'a> {
    world: &'a World,
    address: usize,
    remaining: std::cell::Cell<u32>,
    reads: std::cell::Cell<u32>,
}

impl engine_native_memory::Memory for Faulty<'_> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        if address == self.address {
            self.reads.set(self.reads.get() + 1);
            if self.remaining.get() != 0 {
                self.remaining.set(self.remaining.get() - 1);
                return false;
            }
        }
        self.world.copy(address, out)
    }
}

#[test]
fn a_one_shot_drift_fault_publishes_healed_slice_rows() {
    drift_fault_slice(true);
}

#[test]
fn a_persistent_drift_fault_keeps_the_slice_unavailable() {
    drift_fault_slice(false);
}

fn drift_fault_slice(once: bool) {
    use engine_native_memory::batch_fallback::{Batch, count};
    let mut world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    let banked = SECOND.origin + SLOT + engine_research::cresearch_slot::BANKED_DAYS;
    world.put(banked, 4_200_000_i64.to_le_bytes());
    world.put(
        SECOND.origin + TECH + engine_research::ctechnology::LIVE_PROGRESS,
        270_000_i64.to_le_bytes(),
    );
    let only = |rows: Research| Research {
        inputs: rows
            .inputs
            .into_iter()
            .filter(|row| row.slot.country == SECOND.id)
            .collect(),
        bonuses: rows
            .bonuses
            .into_iter()
            .filter(|row| row.country == SECOND.id)
            .collect(),
        modifiers: rows
            .modifiers
            .into_iter()
            .filter(|row| row.country == SECOND.id)
            .collect(),
    };
    let expected = only(reference(&world));
    let faulty = Faulty {
        world: &world,
        address: banked,
        remaining: std::cell::Cell::new(if once { 1 } else { u32::MAX }),
        reads: std::cell::Cell::new(0),
    };
    let before = count(Batch::GovernanceResearchDrift);
    let captured = slice_capture(&faulty, 27, &mut fingerprints, &changes);
    if once {
        assert_eq!(captured.research_inputs.outcome, Outcome::Rows { len: 1 });
        let rows = research(&captured);
        assert_eq!(rows, expected);
        assert_eq!(rows.inputs[0].slot.banked_days_raw, 4_200_000);
        assert_eq!(rows.inputs[0].progress_raw, 270_000);
    } else {
        assert!(matches!(
            captured.research_inputs.outcome,
            Outcome::Unavailable { .. }
        ));
    }
    assert_eq!(count(Batch::GovernanceResearchDrift) - before, 1);
    assert_eq!(faulty.reads.get(), 2);
    assert!(!changes.poisoned(Table::Research));
}

fn outcomes(captured: &Captured) -> [Outcome; 3] {
    [
        captured.research_inputs.outcome,
        captured.research_bonuses.outcome,
        captured.research_modifiers.outcome,
    ]
}

#[test]
fn a_carried_slice_fails_as_the_full_walk_at_each_fault() {
    let status = FIRST.origin + STATUS;
    let mut faults = vec![
        FIRST.origin + ccountry::TECHNOLOGY_STATUS,
        status,
        status + ctechnology_status::COUNTRY,
        status + ctechnology_status::RESEARCH_SLOTS_DATA,
    ];
    for (position, slot) in FIRST.slots.iter().enumerate() {
        faults.push(
            FIRST.origin + SLOT + position * 0x100 + engine_research::cresearch_slot::BANKED_DAYS,
        );
        if slot.tech.is_some() {
            faults.push(
                FIRST.origin
                    + TECH
                    + position * 0x400
                    + engine_research::ctechnology::LIVE_PROGRESS,
            );
        }
    }
    let world = World::new();
    for address in faults {
        let faulty = |remaining| Faulty {
            world: &world,
            address,
            remaining: std::cell::Cell::new(remaining),
            reads: std::cell::Cell::new(0),
        };
        let full = {
            let memory = faulty(u32::MAX);
            let mut captured = captured();
            let mut part = captured.take_research();
            part.capture(&memory, context(0, true), &mut SliceFingerprints::new());
            captured.restore_research(part);
            outcomes(&captured)
        };
        assert!(
            matches!(full[0], Outcome::Unavailable { .. }),
            "{address:#x}: {full:?}"
        );
        let changes = Domain::enabled(64);
        let mut fingerprints = primed(&world, &changes);
        let memory = faulty(u32::MAX);
        let carried = slice_capture(&memory, 25, &mut fingerprints, &changes);
        assert_eq!(outcomes(&carried), full, "{address:#x}");
        let drift = address >= FIRST.origin + SLOT;
        assert_eq!(
            memory.reads.get(),
            if drift { 2 } else { 1 },
            "{address:#x} fallback not bounded"
        );
    }
}
