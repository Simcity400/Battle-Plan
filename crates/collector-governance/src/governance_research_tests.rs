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
use crate::country_input_changes::{Domain, Table};
use engine_native::{cgame_state, modifier_definition};
use engine_research::{climited_use_tech_bonus as bonus, research_inputs, research_modifiers};

const BASE: usize = 0;
const HEAP_START: usize = 0x1000;
const HEAP_END: usize = 0x3_0000;
const GAME_STATE: usize = 0x1000;
const COUNTRY_TABLE: usize = 0x2000;
const LINK_TABLE: usize = 0x2100;
/// Modifier-definition rows. Key 0xcd's row sits at index 0xcd, inside the heap.
const METADATA: usize = 0x3000;
/// The module data window. It is placed from a public engine RVA, and
/// `World::new` checks that it also covers the research modifier
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
    /// The technology category-speed sum in the modifier walk.
    category_speed: i64,
    /// The template's XP modifier, present when the technology uses experience.
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

impl engine_native::Memory for World {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        // The module window first: a scaled heap reaches past its start.
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

    /// A one-entry sorted accumulator: key, then raw value.
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
        // Each country owns a state, so its research drifts daily.
        self.put(o + ccountry::OWNED_STATE_COUNT, 1_i32.to_le_bytes());
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
            // The child wrapper holds an accumulator whose string names the source.
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
/// order, each into fresh buffers grown from the observed inventory demand,
/// and every per-country outcome must be complete.
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
            engine_native::walk::Buffer::Growing(&mut modifiers),
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
    assert_eq!((count(1), count(3)), (23, 18));
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
fn gap_research_grants_fingerprints_invalidate_for_every_inventory_kind() {
    use research_modifiers::{Kind, Row};
    for kind in [
        Kind::GrantCounter,
        Kind::SpeedGrant,
        Kind::CostGrant,
        Kind::GrantUses,
        Kind::GrantTechnology,
        Kind::GrantCategory,
    ] {
        let original = Row {
            kind,
            index: research_modifiers::grants::INVENTORY_INDEX,
            raw: 12,
            auxiliary: 34,
            count: 2,
            ..Default::default()
        };
        let before = research_digest(&[], &[], std::slice::from_ref(&original));
        for field in 0..3 {
            let mut changed = original.clone();
            match field {
                0 => changed.raw += 1,
                1 => changed.auxiliary += 1,
                _ => changed.count += 1,
            }
            assert_ne!(
                before,
                research_digest(&[], &[], &[changed]),
                "{kind:?} field {field}"
            );
        }
    }
}

#[test]
fn seed_then_bitmap_installation_keeps_grant_inventory_shape_and_certificate() {
    use crate::country_input_changes::{Changes, Mismatch};
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
        fn mismatch(&self, table: Table, at: Mismatch) {
            self.domain.mismatch(table, at);
        }
        fn every(&self, _: Table) -> bool {
            true
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
        (2, false, true, true),
    ] {
        changes.installed.store(installed, Ordering::Relaxed);
        part.capture_changed(&world, context(hour, forced), &mut fingerprints, &changes);
        assert_eq!(part.changed.certified, certified, "hour {hour}");
        assert_eq!(part.changed.mismatches, 0, "hour {hour}");
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
    assert_eq!(part.changed.verified, 2);
}

#[test]
fn research_diagnostics_keep_all_list_counts_and_each_modifier_kind_separate() {
    use research_modifiers::{Kind, Row};
    let initial = [
        Row {
            kind: Kind::GrantCounter,
            raw: 7,
            ..Default::default()
        },
        Row {
            kind: Kind::GrantUses,
            raw: 3,
            ..Default::default()
        },
    ];
    let before = research_digest(&[], &[], &initial);
    let mut changed = initial.to_vec();
    changed[0].source_id = 8;
    changed[1].raw = 2;
    changed.push(Row {
        kind: Kind::SpeedGrant,
        ..Default::default()
    });
    let mut after = research_digest(&[], &[], &changed);
    // The integrated sharing roster has wire kind 31. A separate count and
    // identity/value pair must survive simultaneous grant differences.
    let roster = MODIFIER_GROUPS + 31 * 3;
    after[0] = 4;
    after[1] = 2;
    after[2] += 1;
    after[roster] = 1;
    after[roster + 1] ^= 1;
    after[roster + 2] ^= 1;
    let (field, detail) = research_mismatch(Some(&before), &after, true, true, false, true);
    assert_eq!(field, "ResearchInputs row count");
    assert_eq!(detail.previous_counts, Some([0, 0, 2]));
    assert_eq!(detail.current_counts, [4, 2, 4]);
    let old = detail.previous_modifier_counts.unwrap();
    let new = detail.current_modifier_counts;
    assert_eq!(
        (
            old[Kind::SpeedGrant as usize],
            new[Kind::SpeedGrant as usize]
        ),
        (0, 1)
    );
    assert_eq!((old[31], new[31]), (0, 1));
    assert_eq!(
        detail.modifier_identity_differences,
        (1 << Kind::GrantCounter as u32) | (1 << Kind::SpeedGrant as u32) | (1 << 31)
    );
    assert_eq!(
        detail.modifier_value_differences,
        (1 << Kind::GrantUses as u32) | (1 << Kind::SpeedGrant as u32) | (1 << 31)
    );
    assert!(detail.take_available && detail.certified && detail.selection_force_full);
    assert!(!detail.force_full);
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

/// Country IDs of the research rows a capture produced.
fn owners(captured: &Captured) -> Vec<u32> {
    let mut owners: Vec<_> = research(captured)
        .inputs
        .iter()
        .map(|r| r.slot.country)
        .collect();
    owners.dedup();
    owners
}

/// One hour's research read, selected by `changes`' marks, with the hour's
/// sampled comparisons ([`crate::sampled_verify::begin_hour`]).
fn slice_capture(
    world: &impl engine_native::Memory,
    hour: i32,
    fingerprints: &mut SliceFingerprints,
    changes: &Domain,
) -> Captured {
    crate::sampled_verify::begin_hour(hour);
    let mut captured = captured();
    let mut part = captured.take_research();
    part.capture_changed(world, context(hour, false), fingerprints, changes);
    captured.restore_research(part);
    captured
}

/// A full-hour read: every country is read, and the take's epoch held.
fn primed(world: &impl engine_native::Memory, changes: &Domain) -> SliceFingerprints {
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
    // Country 1 is slice 1, country 3 slice 3; hour 25 is slot 1 again.
    for (hour, expected) in [(1, vec![1]), (3, vec![3]), (5, vec![]), (25, vec![1])] {
        let captured = slice_capture(&world, hour, &mut fingerprints, &changes);
        assert_eq!(owners(&captured), expected, "hour {hour}");
        let selection = captured.research_selection();
        assert_eq!(selection.hour, hour);
        assert_eq!(selection.early_count, 0);
        assert_eq!(selection.scanned, 2);
        assert_eq!(selection.contains(1), hour % 24 == 1);
        assert_eq!(selection.contains(3), hour % 24 == 3);
        // Every emitted row belongs to a selected owner; the spool records
        // their read hour in SliceReads.
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
    // The slice rows, carried from the held read, equal that country's rows
    // from a full read.
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

/// Test E: a country a hooked writer marked is read within the hour, alone;
/// an unmarked one waits for its slice, which carries its held read with the
/// daily progress read afresh. An unmarked change of a certified field is a
/// missed writer: the slice whose sampled comparison covers the country
/// reads it whole and disables reuse, and every country is read from then
/// on.
#[test]
fn a_marked_country_is_read_within_the_hour_and_an_unmarked_change_poisons_at_its_slice() {
    let mut world = World::new();
    let changes = Domain::enabled(64);
    // Nothing held yet: every country is read, outside its slice.
    let mut fresh = SliceFingerprints::new();
    assert_eq!(
        owners(&slice_capture(&world, 5, &mut fresh, &changes)),
        [1, 3]
    );
    // After a full hour, a quiet hour reads nobody outside the slice.
    let mut fingerprints = primed(&world, &changes);
    assert!(owners(&slice_capture(&world, 5, &mut fingerprints, &changes)).is_empty());
    // Technology assignment writes country 3's slot speed and marks it.
    let speed = SECOND.origin + SLOT + engine_research::cresearch_slot::RESEARCH_SPEED;
    world.put(speed, 95_000_i64.to_le_bytes());
    changes.write(Table::Research, 3);
    let captured = slice_capture(&world, 6, &mut fingerprints, &changes);
    assert_eq!(owners(&captured), [3]);
    let selection = captured.research_selection();
    assert_eq!(selection.early_count, 1);
    assert!(selection.contains(3) && !selection.contains(1));
    assert!(!research(&captured).modifiers.is_empty());
    // Taken: the next hour does not read it again.
    assert!(owners(&slice_capture(&world, 7, &mut fingerprints, &changes)).is_empty());
    // Daily progress moves unmarked: no early read, and country 3's slice
    // (hour 27) reads it and compares equal.
    world.put(
        SECOND.origin + TECH + engine_research::ctechnology::LIVE_PROGRESS,
        260_000_i64.to_le_bytes(),
    );
    assert!(owners(&slice_capture(&world, 8, &mut fingerprints, &changes)).is_empty());
    let captured = slice_capture(&world, 27, &mut fingerprints, &changes);
    assert_eq!(owners(&captured), [3]);
    assert_eq!(research(&captured).inputs[0].progress_raw, 260_000);
    assert!(!changes.poisoned(Table::Research));
    // A slot speed changed with no mark is not read early, and the next
    // slice carries the held speed.
    world.put(speed, 97_000_i64.to_le_bytes());
    assert!(owners(&slice_capture(&world, 28, &mut fingerprints, &changes)).is_empty());
    let carried = slice_capture(&world, 51, &mut fingerprints, &changes);
    assert_eq!(owners(&carried), [3]);
    assert_eq!(research(&carried).inputs[0].slot.research_speed_raw, 95_000);
    assert!(!changes.poisoned(Table::Research));
    // The slice whose sampled comparison covers country 3 reads it whole,
    // finds the change and disables reuse.
    assert_eq!(
        owners(&slice_capture(&world, 3 + 576, &mut fingerprints, &changes)),
        [3]
    );
    assert!(changes.poisoned(Table::Research));
    // The status names the country and the field group that differed.
    let first = changes.mismatches(Table::Research).first.unwrap();
    assert_eq!(
        (first.country, first.field),
        (3, "ResearchInputs slot research_speed_raw")
    );
    assert_eq!(changes.mismatches(Table::Research).last, Some(first));
    assert_eq!(
        owners(&slice_capture(&world, 4 + 576, &mut fingerprints, &changes)),
        [1, 3]
    );
}

/// A writer that names no country, a moved epoch (a load or gap) or an
/// interrupted job reads every country at the next hour, then settles.
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
    // A stability writer leaves research alone.
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
    /// `countries` copies of the two specs at 0x8000-byte origins, every
    /// `stated`-th one owning a state, census slot and raw id equal to its
    /// native id.
    fn scaled(countries: usize, stated: usize) -> Self {
        const STRIDE: usize = 0x8000;
        let first = 0x1_0000;
        // Objects skip the module window, which hides the heap bytes it
        // overlaps.
        let origin = |index: usize| {
            let origin = first + index * STRIDE;
            if origin + STRIDE > IMAGE_START {
                origin + IMAGE_LEN.next_multiple_of(STRIDE) + STRIDE
            } else {
                origin
            }
        };
        let tables = origin(countries);
        let mut world = Self::new();
        // Drop the two fixture countries; keep the census header and the
        // metadata rows below them.
        world.heap.truncate(first - HEAP_START);
        world
            .heap
            .resize(tables + (countries + 1) * 12 - HEAP_START, 0);
        let country_table = tables;
        let link_table = tables + (countries + 1) * 8;
        world.array(
            GAME_STATE + cgame_state::COUNTRIES,
            country_table,
            countries as u32 + 1,
        );
        world.array(
            GAME_STATE + cgame_state::COUNTRY_LINKS,
            link_table,
            countries as u32 + 1,
        );
        for index in 0..countries {
            let id = index as u32 + 1;
            let template = if index % 2 == 0 { &FIRST } else { &SECOND };
            let spec = CountrySpec {
                census_slot: id,
                id,
                origin: origin(index),
                slots: template.slots,
                bonuses: template.bonuses,
                country_speed: template.country_speed,
                source: template.source,
            };
            world.put(
                country_table + id as usize * 8,
                (spec.origin as u64).to_le_bytes(),
            );
            world.put(link_table + id as usize * 4, id.to_le_bytes());
            world.country(&spec);
            let owned = i32::from(index % stated == 0);
            world.put(
                spec.origin + ccountry::OWNED_STATE_COUNT,
                owned.to_le_bytes(),
            );
        }
        world
    }
}

/// Quiet-hour cost of the research job before and after the drift filter,
/// over a synthetic world of 446, 1,500 and 5,000 countries: "all" has every
/// country owning a state (the reads of the old selection), "1/6" a sixth,
/// as in a vanilla census; "roster" reads through the hour's authenticated
/// country census, as the live job does (`Cache::roster_view`). No writer
/// marks anything; each run is one full hour, then 24 quiet hours.
/// `cargo test --release -p collector research_quiet_hours_bench -- --ignored --nocapture`
#[test]
#[ignore = "bench"]
fn research_quiet_hours_bench() {
    fn run(world: &impl engine_native::Memory, countries: usize, label: &str) {
        let changes = Domain::enabled(countries + 64);
        let mut fingerprints = primed(world, &changes);
        let mut reads = 0;
        let mut samples = Vec::new();
        for round in 0..5 {
            let started = std::time::Instant::now();
            for hour in 1..=24 {
                let captured = slice_capture(world, round * 24 + hour, &mut fingerprints, &changes);
                if round == 0 {
                    reads += owners(&captured).len();
                }
            }
            samples.push(started.elapsed().as_secs_f64() * 1e3 / 24.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{countries} countries, {label}: {:.1} reads/h, {:.3} ms/h median ({:.3}..{:.3})",
            reads as f64 / 24.0,
            samples[2],
            samples[0],
            samples[4]
        );
        assert!(!changes.poisoned(Table::Research));
    }
    for countries in [446, 1_500, 5_000] {
        for (label, stated) in [("all owning a state", 1), ("1/6 owning a state", 6)] {
            let world = World::scaled(countries, stated);
            run(&world, countries, label);
            let mut cache = engine_native::country_memory::Cache::try_with_capacity(
                engine_native::country_memory::Cache::cells_for_countries(countries + 1, 64),
            )
            .unwrap();
            let seed = cache.prepare(&world, BASE, GAME_STATE).seed_status();
            assert_eq!(seed, engine_native::country_memory::SeedStatus::Ready);
            let roster = cache.roster_view(&world);
            assert!(
                engine_native::Memory::authenticated_countries(&roster, BASE, GAME_STATE).is_some(),
                "the roster view serves the census"
            );
            run(&roster, countries, &format!("{label}, roster"));
        }
    }
}

/// The addresses a capture read, over a [`World`].
struct Reads<'a> {
    world: &'a World,
    addresses: std::cell::RefCell<Vec<usize>>,
}

impl engine_native::Memory for Reads<'_> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.addresses.borrow_mut().push(address);
        self.world.copy(address, out)
    }
}

/// A drifting country no writer marked is carried at its slice: of its
/// research only its status, slot roster, banked days and assigned
/// technologies' progress are read, and its rows equal a full read of the
/// same memory. Before, the slice read every input, bonus and modifier
/// source again.
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
        SECOND.origin + ccountry::MODIFIER_ACCUMULATOR
            ..SECOND.origin + ccountry::MODIFIER_ACCUMULATOR + 0x60,
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
    // A slot roster that moved without a mark is read whole, and its
    // comparison finds the missed write.
    world.array(
        SECOND.origin + STATUS + ctechnology_status::RESEARCH_SLOTS_DATA,
        SECOND.origin + SLOTS,
        0,
    );
    slice_capture(&world, 51, &mut fingerprints, &changes);
    assert!(changes.poisoned(Table::Research));
}

/// Country 1's roster lists its slots in native index order 2, 1, 0, and
/// its rows are sorted by slot index: a carried slice puts each slot's
/// banked days on that slot's row, as a full read does.
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

/// [`World`] memory that fails reads of `address` while `remaining` lasts.
struct Faulty<'a> {
    world: &'a World,
    address: usize,
    remaining: std::cell::Cell<u32>,
    reads: std::cell::Cell<u32>,
}

impl engine_native::Memory for Faulty<'_> {
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

/// The relaxed read-order rule lets the optional gather consume a one-shot
/// fault: its one original serial pass must publish the now-readable live
/// rows. A persistent fault still refuses at the original scalar boundary.
#[test]
fn a_one_shot_drift_fault_publishes_healed_slice_rows() {
    drift_fault_slice(true);
}

#[test]
fn a_persistent_drift_fault_keeps_the_slice_unavailable() {
    drift_fault_slice(false);
}

fn drift_fault_slice(once: bool) {
    use engine_native::{batch_fallback::Batch, batch_fallback::count};
    let mut world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    let banked = SECOND.origin + SLOT + engine_research::cresearch_slot::BANKED_DAYS;
    // Distinguish successful live fallback rows from stale held rows.
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
    assert_eq!(faulty.reads.get(), 2); // Failed gather plus one original serial pass.
    assert!(!changes.poisoned(Table::Research));
}

/// Country 1's held research carried at its slice reads each slot's banked
/// days in roster order (positions 0, 1, 2; native indices 2, 1, 0), as the
/// full walk acquires them, not in its rows' index order.
#[test]
fn a_carried_slice_reads_the_slots_in_roster_order() {
    struct Banked<'a> {
        world: &'a World,
        reads: std::cell::RefCell<Vec<usize>>,
    }
    impl engine_native::Memory for Banked<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            let first = FIRST.origin + SLOT;
            if (first..first + FIRST.slots.len() * 0x100).contains(&address)
                && (address - first) % 0x100 == engine_research::cresearch_slot::BANKED_DAYS
            {
                self.reads.borrow_mut().push((address - first) / 0x100);
            }
            self.world.copy(address, out)
        }
    }
    let world = World::new();
    let changes = Domain::enabled(64);
    let mut fingerprints = primed(&world, &changes);
    let banked = Banked {
        world: &world,
        reads: Default::default(),
    };
    let carried = slice_capture(&banked, 25, &mut fingerprints, &changes);
    assert_eq!(owners(&carried), [1]);
    assert_eq!(banked.reads.into_inner(), [0, 1, 2]);
}

/// The three research tables' outcomes of one capture.
fn outcomes(captured: &Captured) -> [Outcome; 3] {
    [
        captured.research_inputs.outcome,
        captured.research_bonuses.outcome,
        captured.research_modifiers.outcome,
    ]
}

/// A persistent fault at each operand a carried slice of country 1 reads (its
/// status pointer and identity, slot roster header, each slot's banked days
/// and each assigned technology's progress) gives the carried slice exactly
/// the outcomes a full read meets at the same fault: the same tables
/// unavailable with the same reasons.
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
