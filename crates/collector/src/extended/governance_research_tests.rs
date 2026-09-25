//! The loaned research part against a synthetic world whose research walks
//! all succeed with rows, so a wrong split cannot pass by failing both sides.
//!
//! The world has a three-slot country census: slot 1 holds country id 1, slot 2
//! is null and slot 3 holds country id 3. Both countries have an authenticated
//! CTechnologyStatus with active research, and each has one applied
//! limited-use bonus. Country 1 has a direct research-speed modifier and
//! country 3 has a named source in its modifier tree. The two countries use
//! different technologies, progress, bonuses and modifiers, so rows given the
//! wrong country, placed in the wrong order or overwritten show up in the
//! row-for-row comparison.
//!
//! The reference calls the engine's per-country walks directly, one country
//! at a time, in census order, each into fresh buffers. It shares none of
//! `ResearchPart`'s buffer slicing or outcome accumulation.
//!
//! Memory is a sparse overlay, not one flat image. A small heap window holds
//! the objects, and a 64 KiB module-data window, zero unless written, serves
//! the research cost, research modifier and modifier-definition defines. The
//! module base is zero, as in the engine research tests, so each vtable
//! identity is its bare RVA. Addresses outside both windows are unreadable,
//! including the null page.

use super::*;
use engine::{
    cgame_state, climited_use_tech_bonus as bonus, modifier_definition, research_inputs,
    research_modifiers,
};

const BASE: usize = 0;
const HEAP_START: usize = 0x1000;
const HEAP_END: usize = 0x3_0000;
const GAME_STATE: usize = 0x1000;
const COUNTRY_TABLE: usize = 0x2000;
const LINK_TABLE: usize = 0x2100;
/// Modifier-definition rows. Key 0xcd's row sits at +0xcd*STRIDE, inside the heap.
const METADATA: usize = 0x3000;
/// The module data window. It is placed from a public engine RVA, and
/// `World::new` checks that it also covers the private research modifier
/// defines by requiring `research_modifiers::Context::read` to succeed.
const IMAGE_START: usize = modifier_definition::TABLE_RVA & !0xffff;
const IMAGE_LEN: usize = 0x1_0000;

const COST_SCALE: i64 = 130_000;
const AHEAD_PENALTY: i64 = 175_000;
const REASON: &str = "governance acquisition panicked";

/// One assigned technology and the objects that only it owns.
struct Tech {
    handle: u32,
    database_index: i32,
    token: &'static [u8],
    year: i32,
    progress: i64,
    cost: i64,
    applied_speed: i64,
    /// CTechnology +0x190, the category-speed sum in the modifier walk.
    category_speed: i64,
    /// CTechnology +0x1e8 flag; when set, the template's +0x538 XP modifier.
    experience: Option<i64>,
    bonus_ids: &'static [i32],
}

/// One research slot, listed in native roster order. `index` is the native slot index.
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
    /// Raw 0xcd research-speed entry of the country accumulator.
    country_speed: i64,
    /// A named child source in the accumulator tree (name, raw 0xcd).
    source: Option<(&'static [u8], i64)>,
}

// Per-country object layout relative to its origin (each origin owns 0x1_0000 bytes).
const STATUS: usize = 0x1000;
const SLOTS: usize = 0x1200;
const SLOT: usize = 0x1300; // + i * 0x100
const TECH: usize = 0x2000; // + i * 0x400
const TEMPLATE: usize = 0x3000; // + i * 0x800
const IDS: usize = 0x4000; // + i * 0x40
const BONUS_ROSTER: usize = 0x4200;
const BONUS: usize = 0x4400; // + j * 0x100
const STORAGE: usize = 0x5000;
const CHILDREN: usize = 0x5100;
const WRAPPER: usize = 0x5200;
const CHILD_STORAGE: usize = 0x5400;

/// Country id 1. Its roster is permuted: native slot index 2 comes first,
/// idle slot 1 second and slot 0 last. The walk sorts by native index.
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

/// Country id 3, in census slot 3, after the null slot 2.
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

impl engine::Memory for World {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        let source = window(&self.heap, HEAP_START, address, out.len())
            .or_else(|| window(&self.image, IMAGE_START, address, out.len()));
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
        // Country census: [sentinel, id 1, null, id 3]; links map raw id -> slot.
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
        // Module data: research cost defines, the modifier-definition table
        // and the ahead-penalty define. The other research modifier defines
        // read as zero in the zero-filled window.
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

    fn image_put(&mut self, rva: usize, bytes: &[u8]) {
        let at = BASE + rva - IMAGE_START;
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

    /// Inline MSVC string (capacity 15).
    fn text(&mut self, at: usize, text: &[u8]) {
        assert!(text.len() <= 15);
        self.heap[at - HEAP_START..at - HEAP_START + text.len()].copy_from_slice(text);
        self.put(at + 16, (text.len() as u64).to_le_bytes());
        self.put(at + 24, 15_u64.to_le_bytes());
    }

    /// A one-entry sorted accumulator: key at +0, raw value at +8.
    fn modifier(&mut self, accumulator: usize, storage: usize, key: u32, raw: i64) {
        self.array(accumulator, storage, 1);
        self.put(storage, key.to_le_bytes());
        self.put(storage + 8, raw.to_le_bytes());
    }

    fn country(&mut self, spec: &CountrySpec) {
        let o = spec.origin;
        let status = o + STATUS;
        self.put(o, (BASE as u64 + ccountry::VTABLE_RVA as u64).to_le_bytes());
        self.put(o + ccountry::ID, spec.id.to_le_bytes());
        self.pointer(o + ccountry::TECHNOLOGY_STATUS, status);
        self.put(
            status,
            (BASE as u64 + ctechnology_status::VTABLE_RVA as u64).to_le_bytes(),
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
                (BASE as u64 + cresearch_slot::VTABLE_RVA as u64).to_le_bytes(),
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
                (BASE as u64 + ctechnology::VTABLE_RVA as u64).to_le_bytes(),
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
            self.put(at, (BASE as u64 + bonus::VTABLE_RVA as u64).to_le_bytes());
            self.put(at + bonus::ID, value.id.to_le_bytes());
            self.text(at + bonus::NAME, value.name);
            self.put(at + bonus::SPEED_BONUS, value.speed.to_le_bytes());
            self.put(at + bonus::AHEAD_REDUCTION, value.ahead.to_le_bytes());
        }
        let accumulator = o + ccountry::MODIFIER_ACCUMULATOR;
        self.modifier(accumulator, o + STORAGE, 0xcd, spec.country_speed);
        if let Some((name, raw)) = spec.source {
            // accumulator+0x18 children -> wrapper; wrapper+0x10 is the child
            // accumulator, whose +0x48 string names the source.
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

/// Independent reference: the engine walks, country by country in census
/// order, each into its own fresh buffers of the collector's default
/// capacities, and every per-country outcome must be complete.
fn reference(world: &World) -> Research {
    let capacity = Capacities::default();
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
        let mut inputs = vec![research_inputs::Row::default(); capacity.rows[RESEARCH[0]]];
        let mut bonuses = vec![research_inputs::BonusRow::default(); capacity.rows[RESEARCH[1]]];
        let mut modifiers = vec![research_modifiers::Row::default(); capacity.rows[RESEARCH[2]]];
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
        let modifier_outcome = research_modifiers::walk_country(
            world,
            BASE,
            &countries,
            country,
            modifier_context,
            &inputs[..input_len],
            &mut modifiers,
        );
        let Outcome::Rows { len: modifier_len } = modifier_outcome else {
            panic!("country {} modifiers: {modifier_outcome:?}", country.id);
        };
        // Every country contributes to every table, so a dropped or
        // overwritten country block changes the comparison.
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

/// The complete research rows the captured family holds, or a panic that names the incomplete table.
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

/// The loaned part alone, restored into an otherwise untouched family.
fn loaned_only(world: &World) -> Captured {
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture(world, context(0, true), &mut SliceFingerprints::new());
    captured.restore_research(part);
    captured
}

#[test]
fn reference_world_has_distinct_rows_for_both_countries() {
    let world = World::new();
    let expected = reference(&world);
    // Inputs: country 1's three slots sorted by native index, then country 3.
    let inputs: Vec<_> = expected
        .inputs
        .iter()
        .map(|r| (r.slot.country, r.slot.index, r.token.as_bytes().to_vec()))
        .collect();
    assert_eq!(
        inputs,
        [
            (1, 0, b"radio".to_vec()),
            (1, 1, vec![]),
            (1, 2, b"electronic_eng".to_vec()),
            (3, 0, b"infantry_weapon".to_vec()),
        ]
    );
    assert_eq!(expected.inputs[0].progress_raw, 500_000);
    assert_eq!(expected.inputs[3].progress_raw, 250_000);
    assert!(
        expected
            .inputs
            .iter()
            .all(|r| r.cost_scale_raw == COST_SCALE)
    );
    // Both applied bonuses are resolved against their own country's roster.
    let bonuses: Vec<_> = expected
        .bonuses
        .iter()
        .map(|r| {
            (
                r.country,
                r.value.id,
                r.resolved,
                r.value.name.as_bytes().to_vec(),
            )
        })
        .collect();
    assert_eq!(
        bonuses,
        [
            (1, -7, 1, b"bonus_radio".to_vec()),
            (3, -9, 1, b"bonus_infantry".to_vec()),
        ]
    );
    // Modifiers: country-wide operands, per-slot operands and, for country 3, a named source.
    use research_modifiers::Kind;
    let count = |country| {
        expected
            .modifiers
            .iter()
            .filter(|r| r.country == country)
            .count()
    };
    assert_eq!((count(1), count(3)), (20, 15));
    let speed = |country| {
        expected
            .modifiers
            .iter()
            .find(|r| r.country == country && r.kind == Kind::CountrySpeed)
            .unwrap()
            .raw
    };
    assert_eq!((speed(1), speed(3)), (15_000, -12_000));
    let source = expected
        .modifiers
        .iter()
        .find(|r| r.kind == Kind::CountrySource)
        .unwrap();
    assert_eq!(
        (source.country, source.raw, source.name.as_bytes()),
        (3, -12_000, &b"slow_idea"[..])
    );
    assert!(
        expected
            .modifiers
            .iter()
            .any(|r| r.country == 1 && r.kind == Kind::Experience && r.raw == 100_003)
    );
}

/// Test A: the loaned part produces exactly the independent per-country walk.
#[test]
fn research_part_rows_match_independent_per_country_walks() {
    let world = World::new();
    let expected = reference(&world);
    let captured = loaned_only(&world);
    let actual = research(&captured);
    assert_eq!(actual.inputs.len(), 4);
    assert_eq!(actual.bonuses.len(), 2);
    assert_eq!(actual.modifiers.len(), 35);
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

/// Test B: a family capture with the part loaned, in either completion
/// order, leaves research to the part and yields Test A's tables.
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

/// Test C: governance failing does not take the independently acquired research rows with it.
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
        assert!(replay::cadence::should_walk(
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

/// Test D: a failed research part publishes its reason for walked tables and
/// the cadence skip for tables not due. The research tables are currently
/// walked every hour, so only the first case occurs. The expectation is
/// computed from `should_walk` so a later cadence change is still checked.
#[test]
fn research_part_failure_publishes_reason_or_cadence_skip() {
    let world = World::new();
    // One family reused across hours, as the collector's pooled frames are.
    let mut captured = captured();
    for force_full in [false, true] {
        for hour in 0..48 {
            let context = context(hour, force_full);
            let mut part = captured.take_research();
            captured.capture(&world, BASE, GAME_STATE, hour, force_full);
            let family = captured.outcomes();
            // Rows from a successful walk must not survive the later failure.
            part.capture(&world, context, &mut SliceFingerprints::new());
            part.fail(context, "research acquisition panicked");
            captured.restore_research(part);
            let outcomes = captured.outcomes();
            let tables = captured.tables();
            for index in RESEARCH {
                let reason = if replay::cadence::should_walk(TABLES[index], hour, force_full) {
                    "research acquisition panicked"
                } else {
                    replay::cadence::SKIPPED_REASON
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

/// Country IDs of the research rows a capture produced.
fn owners(captured: &Captured) -> Vec<u32> {
    let mut owners: Vec<_> = research(captured).inputs.iter().map(|r| r.slot.country).collect();
    owners.dedup();
    owners
}

fn slice_capture(world: &World, hour: i32, fingerprints: &mut SliceFingerprints) -> Captured {
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture(world, context(hour, false), fingerprints);
    captured.restore_research(part);
    captured
}

/// A full-hour read: every country is read and observed once.
fn primed(world: &World) -> SliceFingerprints {
    let mut fingerprints = SliceFingerprints::new();
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture(world, context(0, true), &mut fingerprints);
    captured.restore_research(part);
    fingerprints
}

#[test]
fn off_full_hours_read_only_the_slice_and_label_rows_with_the_hour() {
    let world = World::new();
    let mut fingerprints = primed(&world);
    // Country 1 is slice 1, country 3 slice 3; hour 25 is slot 1 again.
    for (hour, expected) in [(1, vec![1]), (3, vec![3]), (5, vec![]), (25, vec![1])] {
        let captured = slice_capture(&world, hour, &mut fingerprints);
        assert_eq!(owners(&captured), expected, "hour {hour}");
        let selection = captured.research_selection();
        assert_eq!(selection.hour, hour);
        assert_eq!(selection.early_count, 0);
        assert_eq!(selection.scanned, 2);
        assert_eq!(selection.contains(1), hour % 24 == 1);
        assert_eq!(selection.contains(3), hour % 24 == 3);
        // Every emitted row carries this read's hour.
        for view in research_tables(&captured) {
            let TableView::Complete(rows) = view else {
                panic!("slice tables complete")
            };
            match rows {
                TableRows::ResearchInputs(rows) => assert!(rows.iter().all(|r| r.sample_hour == hour)),
                TableRows::ResearchAppliedBonuses(rows) => {
                    assert!(rows.iter().all(|r| r.sample_hour == hour))
                }
                TableRows::ResearchModifiers(rows) => assert!(rows.iter().all(|r| r.sample_hour == hour)),
                other => panic!("unexpected {:?}", other.table()),
            }
        }
    }
    // The slice rows equal that country's rows from a full read.
    let full = research(&loaned_only(&world));
    let slice = research(&slice_capture(&world, 3, &mut fingerprints));
    let only = |rows: &Research| {
        (
            rows.inputs.iter().filter(|r| r.slot.country == 3).copied().collect::<Vec<_>>(),
            rows.modifiers.iter().filter(|r| r.country == 3).copied().collect::<Vec<_>>(),
        )
    };
    assert_eq!(only(&slice), only(&full));
}

#[test]
fn a_changed_assignment_is_read_early_but_daily_progress_waits_for_the_slice() {
    let mut world = World::new();
    // A country seen for the first time is read at once, outside its slice.
    let mut fresh = SliceFingerprints::new();
    assert_eq!(owners(&slice_capture(&world, 5, &mut fresh)), [1, 3]);
    // After a full hour, nobody is read outside its slice.
    let mut fingerprints = primed(&world);
    assert!(owners(&slice_capture(&world, 5, &mut fingerprints)).is_empty());
    // Daily progress moves: no early read.
    let technology = SECOND.origin + TECH;
    world.put(technology + engine::ctechnology::LIVE_PROGRESS, 260_000_i64.to_le_bytes());
    let captured = slice_capture(&world, 6, &mut fingerprints);
    assert!(owners(&captured).is_empty());
    assert_eq!(captured.research_selection().early_count, 0);
    // A new slot speed (written on assignment) reads country 3 now, in full.
    world.put(SECOND.origin + SLOT + engine::cresearch_slot::RESEARCH_SPEED, 95_000_i64.to_le_bytes());
    let captured = slice_capture(&world, 7, &mut fingerprints);
    assert_eq!(owners(&captured), [3]);
    let selection = captured.research_selection();
    assert_eq!(selection.early_count, 1);
    assert!(selection.contains(3) && !selection.contains(1));
    assert!(!research(&captured).modifiers.is_empty());
    // Seen now: the next hour does not read it again.
    assert!(owners(&slice_capture(&world, 8, &mut fingerprints)).is_empty());
    // After an interrupted job the store is forgotten: every country is new
    // and is read at the next hour, then settles again.
    fingerprints.forget();
    assert_eq!(owners(&slice_capture(&world, 9, &mut fingerprints)), [1, 3]);
    assert!(owners(&slice_capture(&world, 10, &mut fingerprints)).is_empty());
}
