use super::*;
use std::cell::{Cell, RefCell};

const GAME: usize = 0x1000;
const COUNTRY: usize = 0x3000;
const STATUS: usize = 0x5000;
const TECH: usize = 0x6000;
const TEMPLATE: usize = 0x7000;
const METADATA: usize = 0x10000;

struct Fixture {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    calls: Cell<usize>,
    fault: Option<usize>,
}
impl Fixture {
    fn new() -> Self {
        let mut f = Self {
            bytes: vec![0; 0x40000],
            reads: RefCell::new(vec![]),
            calls: Cell::new(0),
            fault: None,
        };
        f.array(GAME + engine_native::cgame_state::COUNTRIES, 0x2000, 3);
        f.array(GAME + engine_native::cgame_state::COUNTRY_LINKS, 0x2100, 3);
        for id in [1_u32, 2] {
            f.put(0x2000 + id as usize * 8, (COUNTRY as u64).to_le_bytes());
            f.put(0x2100 + id as usize * 4, 1_u32.to_le_bytes());
        }
        f.put(COUNTRY, (ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        f.put(COUNTRY + 8, 1_u32.to_le_bytes());
        f.put(
            COUNTRY + ccountry::TECHNOLOGY_STATUS,
            (STATUS as u64).to_le_bytes(),
        );
        f.put(
            STATUS,
            (crate::ctechnology_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        f.put(
            STATUS + crate::ctechnology_status::COUNTRY,
            (COUNTRY as u64).to_le_bytes(),
        );
        f
    }
    fn put<const N: usize>(&mut self, at: usize, bytes: [u8; N]) {
        self.bytes[at..at + N].copy_from_slice(&bytes);
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.put(at, (data as u64).to_le_bytes());
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }
    fn text(&mut self, at: usize, text: &[u8]) {
        assert!(text.len() <= 15);
        self.bytes[at..at + text.len()].copy_from_slice(text);
        self.put(at + 16, (text.len() as u64).to_le_bytes());
        self.put(at + 24, 15_u64.to_le_bytes());
    }
    fn modifier(&mut self, at: usize, storage: usize, key: u32, raw: i64) {
        self.array(at, storage, 1);
        self.put(storage, key.to_le_bytes());
        self.put(storage + 8, raw.to_le_bytes());
    }
    fn field(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, out.len()));
        if self
            .fault
            .is_some_and(|f| at <= f && f < at.saturating_add(out.len()))
        {
            if !out.is_empty() {
                out[0] = 255;
            }
            return false;
        }
        if at == engine_native_parts::modifier_definition::TABLE_RVA.0 && out.len() == 8 {
            out.copy_from_slice(&(METADATA as u64).to_le_bytes());
            return true;
        }
        if let Some(bytes) = self.bytes.get(at..at.saturating_add(out.len())) {
            out.copy_from_slice(bytes);
            true
        } else {
            false
        }
    }
    fn input(&self) -> research_inputs::Row {
        research_inputs::Row {
            technology_pointer: TECH,
            template_pointer: TEMPLATE,
            status_pointer: STATUS,
            template_year: 1938,
            template_index: 1,
            template_max_level: 1,
            slot: crate::cresearch_slot::Row {
                country: 1,
                index: 0,
                technology_handle: 7,
                technology_present: 1,
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn capture(&self, inputs: &[research_inputs::Row], rows: &mut Vec<Row>) -> Outcome {
        let countries = ccountry::Reader::new(self, 0, GAME).unwrap();
        walk_country(
            self,
            0,
            &countries,
            engine_native_memory::country_identity::Country {
                pointer: COUNTRY,
                id: 1,
                canonical_slot: 1,
            },
            Ok(Context {
                minimum: 10_000,
                license: 20_000,
                sharing_maximum: 50_000,
                ahead_factor: 200_000,
            }),
            inputs,
            rows,
        )
    }
}

#[test]
fn gap_research_grants_grant_inventory_captures_unused_kinds_restrictions_and_counter_without_slots()
 {
    use crate::{
        climited_use_tech_bonus as bonus, climited_use_tech_cost_reduction as cost,
        ctechnology_status as status, ctechnology_template as template,
    };
    let mut f = Fixture::new();
    f.put(STATUS + status::NEXT_BONUS_ID, (-7_i32).to_le_bytes());
    f.array(STATUS + status::LIMITED_BONUSES, 0x8000, 1);
    f.array(STATUS + status::COST_REDUCTIONS, 0x8010, 1);
    f.put(0x8000, 0x9000_u64.to_le_bytes());
    f.put(0x8010, 0xa000_u64.to_le_bytes());
    f.put(0x9000, (bonus::VTABLE_RVA.0 as u64).to_le_bytes());
    f.put(0x9000 + bonus::ID, (-3_i32).to_le_bytes());
    f.put(0x9000 + bonus::USES, (-2_i32).to_le_bytes());
    f.put(0x9000 + bonus::CLAIMS, 4_i32.to_le_bytes());
    f.put(0x9000 + bonus::SPEED_BONUS, i64::MIN.to_le_bytes());
    f.put(0x9000 + bonus::AHEAD_REDUCTION, i64::MAX.to_le_bytes());
    f.text(0x9000 + bonus::NAME, b"synthetic grant");
    f.array(0x9000 + bonus::TECHNOLOGIES, 0xb000, 2);
    f.array(0x9000 + bonus::CATEGORIES, 0xb020, 1);
    f.put(0xb000, (TEMPLATE as u64).to_le_bytes());
    f.put(0xb008, (TEMPLATE as u64).to_le_bytes());
    f.put(0xb020, 0xc000_u64.to_le_bytes());
    f.put(TEMPLATE + template::DATABASE_INDEX, 5000_i32.to_le_bytes());
    f.put(
        TEMPLATE + template::TECHNOLOGY_HANDLE,
        0xfedc_ba98_u32.to_le_bytes(),
    );
    f.put(0xc02c, 0xdead_beef_u32.to_le_bytes());
    f.put(0xa000, (cost::VTABLE_RVA.0 as u64).to_le_bytes());
    f.put(0xa000 + cost::ID, 9_i32.to_le_bytes());
    f.put(0xa000 + cost::USES, 8_i32.to_le_bytes());
    f.put(0xa000 + cost::REDUCTION, (-12345_i64).to_le_bytes());
    f.text(0xa000 + cost::NAME, b"cost grant");
    let run = |f: &Fixture, rows: &mut Vec<Row>| {
        let countries = ccountry::Reader::new(f, 0, GAME).unwrap();
        grants::walk_country_buffer(
            f,
            0,
            &countries,
            countries.at(1).unwrap().unwrap(),
            Err("idle needs no modifier defines"),
            &[],
            engine_native_memory::walk::Buffer::Growing(rows),
        )
    };
    let mut rows = Vec::new();
    let outcome = run(&f, &mut rows);
    assert_eq!(outcome, Outcome::Rows { len: 8 });
    let rows = &rows[..outcome.len()];
    assert!(rows.iter().all(|row| row.index == grants::INVENTORY_INDEX));
    assert_eq!((rows[0].kind, rows[0].raw), (Kind::GrantCounter, -7));
    assert_eq!(
        (rows[1].kind, rows[1].raw, rows[1].auxiliary),
        (Kind::SpeedGrant, i64::MIN, i64::MAX)
    );
    assert_eq!((rows[2].raw, rows[2].auxiliary), (-2, 4));
    assert_eq!((rows[3].raw, rows[3].auxiliary), (0xfedc_ba98, 5000));
    assert_eq!(
        rows[3],
        Row {
            ordinal: rows[3].ordinal,
            ..rows[4].clone()
        }
    );
    assert_eq!(rows[5].raw, 0xdead_beef);
    assert_eq!((rows[6].kind, rows[6].raw), (Kind::CostGrant, -12345));
    assert_eq!(
        (rows[7].kind, rows[7].raw, rows[7].count),
        (Kind::GrantUses, 8, 1)
    );
    let cached = research_inputs::BonusRow {
        country: 1,
        source_pointer: 0x9000,
        resolved: 1,
        value: bonus::value(&f, 0x9000, -3).unwrap(),
        ..Default::default()
    };
    f.reads.borrow_mut().clear();
    f.fault = Some(0x9000);
    let countries = ccountry::Reader::new(&f, 0, GAME).unwrap();
    let mut reused = Vec::new();
    let reused_outcome = grants::walk_country_with_applied(
        &f,
        0,
        &countries,
        countries.at(1).unwrap().unwrap(),
        Err("idle needs no modifier defines"),
        &[],
        STATUS,
        &[cached],
        engine_native_memory::walk::Buffer::Growing(&mut reused),
    );
    assert_eq!(reused_outcome, outcome);
    assert_eq!(&reused[..outcome.len()], rows);
    assert!(
        !f.reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == 0x9000
                || *address == STATUS
                || *address == COUNTRY + ccountry::TECHNOLOGY_STATUS
                || *address == 0x9000 + bonus::NAME
                || *address == 0x9000 + bonus::AHEAD_REDUCTION)
    );
    f.fault = Some(0x9000 + bonus::USES);
    assert_eq!(
        run(&f, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "research grant uses unreadable"
        }
    );
}

#[test]
fn gap_research_grants_empty_grant_inventory_is_a_recorded_counter_and_reuses_owned_storage() {
    let f = Fixture::new();
    let countries = ccountry::Reader::new(&f, 0, GAME).unwrap();
    let mut rows = vec![Row::default(); 3];
    let capacity = rows.capacity();
    assert_eq!(
        grants::walk_country_buffer(
            &f,
            0,
            &countries,
            countries.at(1).unwrap().unwrap(),
            Err("unused"),
            &[],
            engine_native_memory::walk::Buffer::Growing(&mut rows)
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].kind, Kind::GrantCounter);
    assert_eq!(rows[0].raw, 0);
    assert_eq!(rows.capacity(), capacity);
}
impl Memory for Fixture {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.field(at, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        output: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        for (i, (&address, out)) in addresses.iter().zip(output).enumerate() {
            if !self.field(address, out) {
                return i;
            }
        }
        N
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        struct Fields<'a>(&'a Fixture);
        impl Memory for Fields<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.0.field(at, out)
            }
        }
        self.calls.set(self.calls.get() + 1);
        Fields(self).copy_records(sources, layout, out)
    }
}

fn capture(f: &Fixture, rows: &mut Vec<Row>) -> Outcome {
    let countries = ccountry::Reader::new(f, 0, GAME).unwrap();
    walk_country(
        f,
        0,
        &countries,
        engine_native_memory::country_identity::Country {
            pointer: COUNTRY,
            id: 1,
            canonical_slot: 1,
        },
        Ok(Context {
            minimum: 10_000,
            license: 20_000,
            sharing_maximum: 50_000,
            ahead_factor: 200_000,
        }),
        &[f.input()],
        rows,
    )
}

#[test]
fn growing_modifier_sources_keep_mod_sized_fanout_text_and_later_growth() {
    const POINTERS: usize = 0x40000;
    const SOURCE: usize = 0x60000;
    const VALUES: usize = 0x60100;
    const NAME: usize = 0x61000;
    let mut f = Fixture::new();
    f.bytes.resize(0x70000, 0);
    f.modifier(
        SOURCE + 0x10,
        VALUES,
        crate::modifier_keys::RESEARCH_SPEED_FACTOR,
        -10_000,
    );
    f.put(SOURCE + 0x58, (NAME as u64).to_le_bytes());
    f.put(SOURCE + 0x68, 2_048_u64.to_le_bytes());
    f.put(SOURCE + 0x70, 2_048_u64.to_le_bytes());
    f.bytes[NAME..NAME + 2_048].fill(b's');
    let mut rows = Vec::new();
    for count in [5_000_u32, 7_000] {
        f.array(
            COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x18,
            POINTERS,
            count,
        );
        for index in 0..count as usize {
            f.put(POINTERS + index * 8, (SOURCE as u64).to_le_bytes());
        }
        f.reads.borrow_mut().clear();
        let result = capture(&f, &mut rows);
        assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
        let sources: Vec<_> = rows[..result.len()]
            .iter()
            .filter(|row| row.kind == Kind::CountrySource)
            .collect();
        assert_eq!(sources.len(), count as usize);
        assert!(
            sources
                .iter()
                .all(|row| row.name.as_bytes() == vec![b's'; 2_048])
        );
        assert_eq!(
            f.reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x18)
                .count(),
            1
        );
    }
}

#[test]
fn growing_source_stack_has_no_chosen_depth_and_keeps_cycles_named() {
    const FIRST: usize = 0x40000;
    const DEPTH: usize = 5_000;
    let mut f = Fixture::new();
    f.bytes.resize(FIRST + DEPTH * 0x100, 0);
    f.array(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x18, 0x8000, 1);
    f.put(0x8000, (FIRST as u64).to_le_bytes());
    for index in 0..DEPTH - 1 {
        let node = FIRST + index * 0x100;
        f.array(node + 0x28, node + 0x80, 1);
        f.put(node + 0x80, ((node + 0x100) as u64).to_le_bytes());
    }
    let mut rows = Vec::new();
    let expected = capture(&f, &mut rows);
    assert!(matches!(expected, Outcome::Rows { .. }));
    assert_eq!(
        f.capture(&[f.input()], &mut vec![Row::default(); expected.len()]),
        expected
    );
    let last = FIRST + (DEPTH - 1) * 0x100;
    f.array(last + 0x28, last + 0x80, 1);
    f.put(last + 0x80, (FIRST as u64).to_le_bytes());
    assert_eq!(
        capture(&f, &mut rows),
        Outcome::Unavailable {
            reason: "research modifier source cycle"
        }
    );
}

#[test]
fn growing_sharing_groups_and_category_rosters_follow_native_counts() {
    const POINTERS: usize = 0x40000;
    const GROUP: usize = 0x50000;
    const CATEGORIES: usize = 0x60000;
    let mut f = Fixture::new();
    f.bytes.resize(CATEGORIES + 5_000 * 40, 0);
    f.array(STATUS + 0x118, POINTERS, 5_000);
    f.put(GROUP, (sharing::FACTION_VTABLE.0 as u64).to_le_bytes());
    for index in 0..5_000 {
        f.put(POINTERS + index * 8, (GROUP as u64).to_le_bytes());
    }
    f.array(TEMPLATE + 0x2c0, CATEGORIES, 5_000);
    for index in 0..5_000 {
        f.text(CATEGORIES + index * 40, b"mod_category");
        f.put(CATEGORIES + index * 40 + 32, (index as u32).to_le_bytes());
    }
    let mut rows = Vec::new();
    let result = capture(&f, &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    assert_eq!(
        rows[..result.len()]
            .iter()
            .filter(|row| row.kind == Kind::Sharing)
            .count(),
        5_000
    );
    assert_eq!(
        rows[..result.len()]
            .iter()
            .filter(|row| row.kind == Kind::SharingCategory)
            .count(),
        5_000
    );
    let mut fixed = crate::allocation::default_vec(rows.len()).unwrap();
    let fixed_result = f.capture(&[f.input()], &mut fixed);
    assert_eq!(fixed_result, result);
    assert_eq!(fixed[..result.len()], rows[..result.len()]);
}

#[test]
fn scientist_categories_batch_the_entire_mod_population() {
    let mut f = Fixture::new();
    f.array(TECH + 0x140, 0x20000, 5_000);
    for index in 0..5_000 {
        f.put(0x20000 + index * 4, (index as u32).to_le_bytes());
    }
    f.put(COUNTRY + ccountry::PROGRAM_STATUS, 0xa000_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0xa010, 1_u32.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    let mut rows = Vec::new();
    let result = capture(&f, &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let categories: Vec<_> = rows[..result.len()]
        .iter()
        .filter(|row| row.kind == Kind::ScientistCategory)
        .collect();
    assert_eq!(categories.len(), 5_000);
    assert_eq!(categories.last().unwrap().source_id, 4_999);
}

#[test]
fn growing_script_sources_have_no_chosen_population_limit() {
    const ENTRIES: usize = 0x40000;
    const SOURCE: usize = 0x80000;
    let mut f = Fixture::new();
    f.bytes.resize(0x81000, 0);
    f.array(TECH + 0x128, ENTRIES, 5_000);
    f.put(SOURCE + 0x30, (-1_234_i64).to_le_bytes());
    for index in 0..5_000 {
        let entry = ENTRIES + index * 40;
        f.put(entry, (SOURCE as u64).to_le_bytes());
        f.text(entry + 8, b"script_source");
    }
    let mut rows = Vec::new();
    let result = capture(&f, &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let scripts: Vec<_> = rows[..result.len()]
        .iter()
        .filter(|row| row.kind == Kind::ScriptSource)
        .collect();
    assert_eq!(scripts.len(), 5_000);
    assert!(
        scripts
            .iter()
            .all(|row| row.raw == -1_234 && row.name.as_bytes() == b"script_source")
    );
}

#[test]
fn active_operands_are_exact_and_idle_avoids_country_dependencies() {
    let mut f = Fixture::new();
    f.put(TECH + 0x190, (-25_001_i64).to_le_bytes());
    f.put(TECH + 0x1a0, 12_345_i64.to_le_bytes());
    f.put(TECH + 0x1e8, [2]);
    f.put(TEMPLATE + 0x538, 100_003_i64.to_le_bytes());
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert_eq!(result, Outcome::Rows { len: 12 });
    let value = |kind| {
        rows[..result.len()]
            .iter()
            .find(|r| r.kind == kind)
            .unwrap()
            .raw
    };
    assert_eq!(value(Kind::CategorySpeed), -25_001);
    assert_eq!(value(Kind::DesignTeam), 12_345);
    assert_eq!(value(Kind::Experience), 100_003);
    assert_eq!(value(Kind::License), 0);
    assert_eq!(value(Kind::Scientist), 0);
    assert!(
        !f.reads
            .borrow()
            .iter()
            .any(|(at, _)| *at == COUNTRY + ccountry::PRODUCTION_STATUS || *at == COUNTRY + 0xfa8)
    );
    let mut idle = f.input();
    idle.slot.technology_present = 0;
    f.reads.borrow_mut().clear();
    assert_eq!(f.capture(&[idle], &mut rows), Outcome::Rows { len: 0 });
    assert_eq!(f.reads.borrow().len(), 2);
}

#[test]
fn scripted_sharing_roster_keeps_idle_countries_duplicates_and_native_extent() {
    const POINTERS: usize = 0x40000;
    const GROUP: usize = 0x50000;
    let mut f = Fixture::new();
    f.bytes.resize(GROUP + 16, 0);
    f.array(STATUS + 0x118, POINTERS, 5_000);
    f.put(GROUP, (sharing::FACTION_VTABLE.0 as u64).to_le_bytes());
    f.put(GROUP + 8, 19_u32.to_le_bytes());
    for index in 0..5_000 {
        f.put(POINTERS + index * 8, (GROUP as u64).to_le_bytes());
    }
    f.fault = Some(GROUP + 12);
    let countries = ccountry::Reader::new(&f, 0, GAME).unwrap();
    let mut input = f.input();
    input.slot.technology_present = 0;
    let mut rows = Vec::new();
    let capture = |record, rows: &mut Vec<Row>| {
        walk_country_buffer_with_roster(
            &f,
            0,
            &countries,
            engine_native_memory::country_identity::Country {
                pointer: COUNTRY,
                id: 1,
                canonical_slot: 1,
            },
            Err("idle country has no modifier context"),
            std::slice::from_ref(&input),
            engine_native_memory::walk::Buffer::Growing(rows),
            record,
        )
    };
    assert_eq!(capture(false, &mut rows), Outcome::Rows { len: 0 });
    let captured = capture(true, &mut rows);
    assert_eq!(captured, Outcome::Rows { len: 5_001 });
    assert_eq!(
        (rows[0].kind, rows[0].raw, rows[0].count),
        (Kind::SharingRoster, -1, 5_000)
    );
    assert!(
        rows[1..captured.len()]
            .iter()
            .all(|row| row.kind == Kind::SharingRoster && row.index == u32::MAX && row.raw == 19)
    );
    assert!(
        !f.reads
            .borrow()
            .iter()
            .any(|&(address, _)| address == COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
    );
}

#[test]
fn cyclic_country_source_trees_fail_without_unbounded_recursion() {
    let mut f = Fixture::new();
    f.array(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x18, 0x8000, 1);
    f.put(
        0x8000,
        (COUNTRY as u64 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR as u64 - 0x10).to_le_bytes(),
    );
    assert_eq!(
        f.capture(&[f.input()], &mut vec![Row::default(); 32]),
        Outcome::Unavailable {
            reason: "research modifier source cycle"
        }
    );
}

#[test]
fn country_tree_is_shared_and_preserves_negative_sources_and_clamps() {
    let mut f = Fixture::new();
    f.modifier(
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x8000,
        0xcd,
        -12_000,
    );
    f.array(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x18, 0x8100, 1);
    f.put(0x8100, 0x8200_u64.to_le_bytes());
    f.modifier(0x8210, 0x8300, 0xcd, -12_000);
    f.text(0x8258, b"slow_idea");
    let mut second = f.input();
    second.slot.index = 1;
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input(), second], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let sources: Vec<_> = rows[..result.len()]
        .iter()
        .filter(|r| r.kind == Kind::CountrySource)
        .collect();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].index, u32::MAX);
    assert_eq!(sources[0].raw, -12_000);
    assert_eq!(sources[0].name.as_bytes(), b"slow_idea");
    f.put(
        METADATA + 0xcd * engine_native_parts::modifier_definition::STRIDE + engine_native_parts::modifier_definition::FLAGS,
        engine_native_parts::modifier_definition::CLAMP_MAXIMUM.to_le_bytes(),
    );
    let result = f.capture(&[f.input()], &mut rows);
    let value = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::CountrySpeed)
        .unwrap();
    assert_eq!((value.raw, value.auxiliary), (-12_000, 0));
    let metadata = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::ModifierMetadata && r.source_id == value.ordinal)
        .unwrap();
    assert_eq!(
        metadata.raw,
        i64::from(engine_native_parts::modifier_definition::CLAMP_MAXIMUM)
    );
}

#[test]
fn every_short_read_refuses_a_complete_breakdown_and_capacity_reports_demand() {
    let mut f = Fixture::new();
    let mut rows = vec![Row::default(); 64];
    let input = f.input();
    f.capture(std::slice::from_ref(&input), &mut rows);
    let reads = f.reads.borrow().clone();
    for (at, _) in reads.into_iter().skip(2) {
        f.fault = Some(at);
        assert!(
            matches!(
                f.capture(std::slice::from_ref(&input), &mut rows),
                Outcome::Unavailable { .. }
            ),
            "{at:x}"
        );
    }
}

#[test]
fn selected_idea_and_script_sources_retain_names_without_reading_catalogues() {
    let mut f = Fixture::new();
    f.array(TECH + 0x110, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(0x8138, (-15_000_i64).to_le_bytes());
    f.put(0x8140, 0x8200_u64.to_le_bytes());
    f.put(0x8200, (engine_native_parts::cidea::VTABLE_RVA.0 as u64).to_le_bytes());
    f.text(0x8218, b"idea_penalty");
    f.array(TECH + 0x128, 0x8400, 1);
    f.put(0x8400, 0x8500_u64.to_le_bytes());
    f.text(0x8408, b"script_bonus");
    f.put(0x8530, 25_000_i64.to_le_bytes());
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::IdeaSource
                && r.raw == -15_000
                && r.name.as_bytes() == b"idea_penalty")
    );
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::ScriptSource
                && r.raw == 25_000
                && r.name.as_bytes() == b"script_bonus")
    );
}

fn walk_buffer(
    f: &Fixture,
    input: &research_inputs::Row,
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let countries = ccountry::Reader::new(f, 0, GAME).unwrap();
    walk_country_buffer(
        f,
        0,
        &countries,
        engine_native_memory::country_identity::Country {
            pointer: COUNTRY,
            id: 1,
            canonical_slot: 1,
        },
        Ok(Context {
            minimum: 10_000,
            license: 20_000,
            sharing_maximum: 50_000,
            ahead_factor: 200_000,
        }),
        std::slice::from_ref(input),
        output,
    )
}

#[test]
fn idea_text_past_512_bytes_is_whole_in_exact_and_count_walks() {
    const NAME: usize = 0x30000;
    let mut f = Fixture::new();
    f.array(TECH + 0x110, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(0x8138, (-15_000_i64).to_le_bytes());
    f.put(0x8140, 0x8200_u64.to_le_bytes());
    f.put(0x8200, (engine_native_parts::cidea::VTABLE_RVA.0 as u64).to_le_bytes());
    let descriptor = 0x8200 + engine_native_parts::cidea::TOKEN;
    f.put(descriptor, (NAME as u64).to_le_bytes());
    f.put(descriptor + 16, 2_048_u64.to_le_bytes());
    f.put(descriptor + 24, 2_048_u64.to_le_bytes());
    f.bytes[NAME..NAME + 2_048].fill(b'i');
    let input = f.input();
    let mut grown = Vec::new();
    let expected = f.capture(std::slice::from_ref(&input), &mut grown);
    let Outcome::Rows { len } = expected else {
        panic!("{expected:?}");
    };
    assert!(
        grown[..len]
            .iter()
            .any(|r| r.kind == Kind::IdeaSource && r.name.as_bytes() == [b'i'; 2_048])
    );
    let mut exact = vec![Row::default(); len];
    assert_eq!(
        walk_buffer(&f, &input, engine_native_memory::walk::Buffer::Exact(&mut exact)),
        expected
    );
    assert_eq!(exact[..], grown[..len]);
    assert_eq!(
        walk_buffer(&f, &input, engine_native_memory::walk::Buffer::Count),
        Outcome::Truncated {
            len: 0,
            unread_rows: len as u32
        }
    );
}

#[test]
fn license_and_scientist_use_only_matching_equipment_and_category() {
    let mut f = Fixture::new();
    f.array(TECH + 0x68, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(
        COUNTRY + ccountry::PRODUCTION_STATUS,
        0x9000_u64.to_le_bytes(),
    );
    f.array(0x91b0, 0x9200, 1);
    f.put(0x9200, 0x9300_u64.to_le_bytes());
    f.put(0x96f0, 0x8100_u64.to_le_bytes());
    f.array(TECH + 0x140, 0x9800, 1);
    f.put(0x9800, 17_u32.to_le_bytes());
    f.put(COUNTRY + 0xfa8, 0xa000_u64.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0xa010, 1_u32.to_le_bytes());
    f.put(0xa104, [1]);
    f.put(0xa108, 17_u32.to_le_bytes());
    f.put(0xa110, 8_765_i64.to_le_bytes());
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::License && r.raw == 20_000)
    );
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::ScientistCategory && r.raw == 8_765)
    );
    f.put(0xa108, 18_u32.to_le_bytes());
    let result = f.capture(&[f.input()], &mut rows);
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::ScientistCategory && r.raw == 0)
    );
}

#[test]
fn sharing_resolves_aliases_and_uses_max_level_with_predecessor_gate() {
    let mut f = Fixture::new();
    f.array(STATUS + 0x118, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(0x8100, (sharing::FACTION_VTABLE.0 as u64).to_le_bytes());
    f.put(0x8108, 4_u32.to_le_bytes());
    f.put(0x8110, 10_000_i64.to_le_bytes());
    f.array(0x8118, 0x8200, 1);
    f.put(0x8200, 2_u32.to_le_bytes());
    f.array(0x8130, 0x8300, 1);
    f.put(0x8300, 5_000_i64.to_le_bytes());
    f.array(
        STATUS + crate::ctechnology_status::TECHNOLOGIES_DATA,
        0x8400,
        1,
    );
    f.put(0x8400, (TECH as u64).to_le_bytes());
    f.put(
        TECH,
        (crate::ctechnology::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(TECH + 8, 7_u32.to_le_bytes());
    f.put(TECH + 0x158, (STATUS as u64).to_le_bytes());
    f.put(TECH + 0x160, (TEMPLATE as u64).to_le_bytes());
    f.put(TECH + 0x174, 1_i32.to_le_bytes());
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let row = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::Sharing)
        .unwrap();
    assert_eq!(
        (row.raw, row.auxiliary, row.count, row.source_id),
        (10_000, 5_000, 1, 4)
    );
    f.array(TECH + 0xc8, 0x8500, 1);
    f.put(0x8500, 0x8600_u64.to_le_bytes());
    f.put(0x8770, 6_u32.to_le_bytes());
    let result = f.capture(&[f.input()], &mut rows);
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::SharingPredecessor && r.raw == 6)
    );
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::SharingMember
                && r.raw == 1
                && r.auxiliary == 1
                && r.source_id == 2)
    );
    f.put(TEMPLATE + 0x409, [1]);
    f.fault = Some(TEMPLATE + 0x2c0);
    let result = f.capture(&[f.input()], &mut rows);
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::SharingDoctrine && r.raw == 1)
    );
    assert!(matches!(result, Outcome::Rows { .. }));
    assert!(!rows[..result.len()].iter().any(|r| r.kind == Kind::Sharing));
}

#[test]
fn scientist_efficiency_sources_preserve_character_identity_and_cached_total() {
    let mut f = Fixture::new();
    f.array(TECH + 0x140, 0x9800, 1);
    f.put(0x9800, 17_u32.to_le_bytes());
    f.put(COUNTRY + ccountry::PROGRAM_STATUS, 0xa000_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0xa010, 2_u32.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    f.put(0xa104, [1]);
    f.put(0xa108, 17_u32.to_le_bytes());
    f.put(0xa110, 8_765_i64.to_le_bytes());
    f.array(0xa118, 0xa200, 1);
    f.put(0xa200, 0xb000_u64.to_le_bytes());
    f.put(
        0xb000,
        (crate::cscientist::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0xb008, 0xc000_u64.to_le_bytes());
    f.modifier(0xb058, 0xb200, 0x27a, 50_000);
    f.put(
        0xc000,
        (engine_native::ccharacter::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0xc008, engine_native::ccharacter::REFERENCE_TYPE.to_le_bytes());
    f.put(0xc00c, 91_u32.to_le_bytes());
    f.put(0xc0c0, 0xb000_u64.to_le_bytes());
    f.text(0xc048, b"scientist_name");
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let source = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::ScientistSource)
        .unwrap();
    assert_eq!(
        (source.raw, source.source_id, source.count),
        (50_000, 91, 17)
    );
    assert_eq!(source.name.as_bytes(), b"scientist_name");
    assert_eq!(
        rows[..result.len()]
            .iter()
            .find(|r| r.kind == Kind::ScientistCategory)
            .unwrap()
            .raw,
        8_765
    );
    f.put(0xc0c0, 0xb100_u64.to_le_bytes());
    assert!(matches!(
        f.capture(&[f.input()], &mut rows),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scientist_hash_collisions_keep_each_raw_category_without_summing() {
    let mut f = Fixture::new();
    f.array(TECH + 0x140, 0x9800, 2);
    f.put(0x9800, 17_u32.to_le_bytes());
    f.put(0x9804, 18_u32.to_le_bytes());
    f.put(COUNTRY + ccountry::PROGRAM_STATUS, 0xa000_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0xa010, 1_u32.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    f.put(0xa098, [1]);
    for (offset, category, distance) in [(0_usize, 17_u32, 1_u8), (48, 18, 2)] {
        f.put(0xa104 + offset, [distance]);
        f.put(0xa108 + offset, category.to_le_bytes());
        f.put(0xa110 + offset, i64::MAX.to_le_bytes());
    }
    let mut rows = vec![Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let categories: Vec<_> = rows[..result.len()]
        .iter()
        .filter(|r| r.kind == Kind::ScientistCategory)
        .map(|r| (r.source_id, r.raw))
        .collect();
    assert_eq!(categories, [(17, i64::MAX), (18, i64::MAX)]);
    f.fault = Some(0xa130);
    assert!(matches!(
        f.capture(&[f.input()], &mut rows),
        Outcome::Unavailable { .. }
    ));
}

const OTHERS: u32 = 4_999;
const LINKS: usize = 0x30000;
const COUNTRY2: usize = 0x46000;
const STATUS2: usize = 0x48000;
fn many_member_fixture() -> Fixture {
    let mut f = Fixture::new();
    f.bytes.resize(0x70000, 0);
    let slots = OTHERS + 2;
    f.array(GAME + engine_native::cgame_state::COUNTRY_LINKS, LINKS, slots);
    f.put(LINKS + 4, 1_u32.to_le_bytes());
    for raw in 2..slots as usize {
        f.put(LINKS + raw * 4, 2_u32.to_le_bytes());
    }
    f.put(0x2000 + 2 * 8, (COUNTRY2 as u64).to_le_bytes());
    f.put(COUNTRY2, (ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    f.put(COUNTRY2 + 8, 2_u32.to_le_bytes());
    f.put(
        COUNTRY2 + ccountry::TECHNOLOGY_STATUS,
        (STATUS2 as u64).to_le_bytes(),
    );
    f.put(
        STATUS2,
        (crate::ctechnology_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(
        STATUS2 + crate::ctechnology_status::COUNTRY,
        (COUNTRY2 as u64).to_le_bytes(),
    );
    f.array(
        STATUS + crate::ctechnology_status::TECHNOLOGIES_DATA,
        0x8400,
        1,
    );
    f.put(0x8400, (TECH as u64).to_le_bytes());
    f.put(
        TECH,
        (crate::ctechnology::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(TECH + 8, 7_u32.to_le_bytes());
    f.put(TECH + 0x158, (STATUS as u64).to_le_bytes());
    f.put(TECH + 0x160, (TEMPLATE as u64).to_le_bytes());
    f.put(TECH + 0x174, 1_i32.to_le_bytes());
    f
}

fn member_rows(rows: &[Row]) -> Vec<(Kind, u32)> {
    rows.iter()
        .filter(|r| matches!(r.kind, Kind::SharingMember | Kind::SharingMissingMember))
        .map(|r| (r.kind, r.source_id))
        .collect()
}

fn expected_members() -> Vec<(Kind, u32)> {
    (2..OTHERS + 2)
        .map(|raw| (Kind::SharingMissingMember, raw))
        .chain(std::iter::once((Kind::SharingMember, 1)))
        .collect()
}

#[test]
fn sharing_member_rosters_are_bounded_only_by_their_native_headers() {
    const MEMBERS: usize = 0x36000;
    const BONUSES: usize = 0x3b000;
    let mut f = many_member_fixture();
    let count = OTHERS + 1;
    f.array(STATUS + 0x118, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(0x8100, (sharing::FACTION_VTABLE.0 as u64).to_le_bytes());
    f.put(0x8108, 4_u32.to_le_bytes());
    f.put(0x8110, 10_000_i64.to_le_bytes());
    f.array(0x8118, MEMBERS, count);
    f.array(0x8130, BONUSES, count);
    for (index, raw) in (2..OTHERS + 2).chain(std::iter::once(1)).enumerate() {
        f.put(MEMBERS + index * 4, raw.to_le_bytes());
        f.put(BONUSES + index * 8, (index as i64 + 100).to_le_bytes());
    }
    let mut rows = vec![Row::default(); 6_000];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let rows = &rows[..result.len()];
    let group = rows.iter().find(|r| r.kind == Kind::Sharing).unwrap();
    assert_eq!(
        (group.raw, group.auxiliary, group.count),
        (10_000, i64::from(OTHERS) + 100, count)
    );
    assert_eq!(member_rows(rows), expected_members());
}

#[test]
fn faction_member_pointer_rosters_are_bounded_only_by_their_native_headers() {
    const TEMPLATE2: usize = 0x49000;
    const FACTION: usize = 0x4a000;
    const DIPLOMACY: usize = 0x4b000;
    const POINTERS: usize = 0x4c000;
    const MEMBERS: usize = 0x56000;
    let mut f = many_member_fixture();
    f.array(STATUS + 0x118, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(
        0x8100,
        (crate::ctechnology_sharing_group::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(0x8108, 9_u32.to_le_bytes());
    f.put(0x8110, 3_000_i64.to_le_bytes());
    f.array(0x8118, 0x8200, 1);
    f.put(0x8200, 1_u32.to_le_bytes());
    f.array(0x8130, 0x8300, 1);
    f.put(0x8300, 2_500_i64.to_le_bytes());
    f.put(0x8148, (TEMPLATE2 as u64).to_le_bytes());
    f.put(TEMPLATE2, 0x029c_11e0_u64.to_le_bytes());
    f.text(TEMPLATE2 + 0x10, b"faction_share");
    f.put(TEMPLATE2 + 0x78, [1]);
    f.put(
        COUNTRY + ccountry::DIPLOMACY_STATUS,
        (DIPLOMACY as u64).to_le_bytes(),
    );
    f.put(
        DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::FACTION,
        (FACTION as u64).to_le_bytes(),
    );
    f.put(
        FACTION,
        (engine_native::cfaction::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    let count = OTHERS + 1;
    f.array(FACTION + engine_native::cfaction::MEMBERS, POINTERS, count);
    for (index, raw) in (2..OTHERS + 2).chain(std::iter::once(1)).enumerate() {
        let member = MEMBERS + index * 16;
        f.put(POINTERS + index * 8, (member as u64).to_le_bytes());
        f.put(member + 8, raw.to_le_bytes());
    }
    let mut rows = vec![Row::default(); 6_000];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let rows = &rows[..result.len()];
    let group = rows.iter().find(|r| r.kind == Kind::Sharing).unwrap();
    assert_eq!(
        (group.raw, group.auxiliary, group.count, group.source_id),
        (3_000, 2_500, count, 9)
    );
    assert_eq!(group.name.as_bytes(), b"faction_share");
    assert_eq!(member_rows(rows), expected_members());
}

#[test]
fn sharing_witness_memo_keeps_its_first_keys_without_changing_rows() {
    const MEMBERS: usize = 0x36000;
    const BONUSES: usize = 0x3b000;
    let run = |roster: &[u32]| {
        let mut f = many_member_fixture();
        f.array(STATUS + 0x118, 0x8000, 1);
        f.put(0x8000, 0x8100_u64.to_le_bytes());
        f.put(0x8100, (sharing::FACTION_VTABLE.0 as u64).to_le_bytes());
        f.array(0x8118, MEMBERS, roster.len() as u32);
        f.array(0x8130, BONUSES, roster.len() as u32);
        for (index, raw) in roster.iter().enumerate() {
            f.put(MEMBERS + index * 4, raw.to_le_bytes());
        }
        let mut rows = vec![Row::default(); 1_000];
        let result = f.capture(&[f.input()], &mut rows);
        assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
        let members: Vec<_> = member_rows(&rows[..result.len()])
            .into_iter()
            .map(|(_, raw)| raw)
            .collect();
        assert_eq!(members, roster);
        f.reads.take().len()
    };
    let distinct: Vec<u32> = (2..602).collect();
    let with = |extra: u32| {
        let mut roster = distinct.clone();
        roster.extend([1, extra]);
        roster
    };
    let mut plain = distinct.clone();
    plain.push(1);
    let baseline = run(&plain);
    assert_eq!(run(&with(2)), baseline);
    assert_eq!(run(&with(600)), baseline);
}

fn batch_inputs(world: &engine_governance_tests::World, id: u32) -> Vec<research_inputs::Row> {
    let mut inputs = vec![research_inputs::Row::default(); 3];
    let mut bonuses = vec![research_inputs::BonusRow::default(); 2];
    assert_eq!(
        research_inputs::walk_country(
            world,
            0,
            engine_governance_tests::World::country(id),
            id,
            research_inputs::Context::read(world, 0),
            &mut inputs,
            &mut bonuses
        )
        .0,
        Outcome::Rows { len: 3 }
    );
    inputs
}
fn batch_capture(
    memory: &engine_governance_tests::Trace<'_, engine_governance_tests::World>,
    id: u32,
    inputs: &[research_inputs::Row],
    serial: bool,
) -> (Outcome, Vec<Row>) {
    use engine_governance_tests::{GAME, World};
    let countries = ccountry::Reader::new(memory, 0, GAME).unwrap();
    let country = engine_native_memory::country_identity::Country {
        pointer: World::country(id),
        id,
        canonical_slot: id,
    };
    let context = Context::read(memory.source, 0);
    memory.clear();
    let mut rows = vec![Row::default(); 64];
    let outcome = if serial {
        super::serial::walk_country_buffer(
            memory,
            0,
            &countries,
            country,
            context,
            inputs,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
        )
    } else {
        walk_country(memory, 0, &countries, country, context, inputs, &mut rows)
    };
    rows.truncate(outcome.len());
    (outcome, rows)
}
fn batch_source_world() -> engine_governance_tests::World {
    use engine_governance_tests::{TECH, World};
    let mut world = World::new(1);
    let country = World::country(1);
    world.array(country + TECH + 0x110, country + 0x7000, 2);
    world.array(country + TECH + 0x128, country + 0x7500, 2);
    for i in 0..2 {
        let contribution = country + 0x7100 + i * 0x100;
        let idea = country + 0x7300 + i * 0x100;
        world.pointer(country + 0x7000 + i * 8, contribution);
        world.put(contribution + 0x38, (-7_001_i64).to_le_bytes());
        world.pointer(contribution + 0x40, idea);
        world.put(idea, (engine_native_parts::cidea::VTABLE_RVA.0 as u64).to_le_bytes());
        world.text(idea + engine_native_parts::cidea::TOKEN, b"mod_idea");
        let script = country + 0x7500 + i * 40;
        let source = country + 0x7600 + i * 0x100;
        world.pointer(script, source);
        world.text(script + 8, b"mod_script");
        world.put(source + 0x30, 9_003_i64.to_le_bytes());
    }
    world
}
#[test]
fn governance_batch_modifier_sources_and_faults_keep_serial_rows_and_reasons() {
    use engine_governance_tests::{TECH, Trace, World, addresses, never_harsher};
    let world = batch_source_world();
    let inputs = batch_inputs(&world, 1);
    let before = Trace::new(&world, "ResearchModifiers sources serial");
    let after = Trace::new(&world, "ResearchModifiers sources batch");
    let old = batch_capture(&before, 1, &inputs, true);
    let new = batch_capture(&after, 1, &inputs, false);
    assert!(matches!(old.0, Outcome::Rows { len } if len > 20));
    assert_eq!(new, old);
    assert_eq!((before.count(), after.count()), (23, 19));
    println!(
        "ResearchModifiers with two idea and script sources: {} -> {} guarded calls/country",
        before.count(),
        after.count()
    );
    let country = World::country(1);
    for (offset, reader) in [
        (TECH + 0x190, "modifier_technologies"),
        (0x7000, "modifier_ideas"),
        (0x7100 + 0x38, "modifier_ideas"),
        (0x7500, "modifier_scripts"),
        (0x7600 + 0x30, "modifier_scripts"),
    ] {
        for once in [false, true] {
            let before = Trace::fault(&world, "Modifiers serial", country + offset, once);
            let after = Trace::fault(&world, "Modifiers batch", country + offset, once);
            let old = batch_capture(&before, 1, &inputs, true);
            let new = batch_capture(&after, 1, &inputs, false);
            if once {
                never_harsher(old.0, new.0);
            } else {
                assert_eq!(new, old);
            }
            assert_eq!(
                &*after.fallbacks.witnesses(),
                &[reader],
                "{reader}, once={once}"
            );
        }
    }
    for address in addresses(&before) {
        for once in [false, true] {
            let before = Trace::fault(&world, "ResearchModifiers serial", address, once);
            let after = Trace::fault(&world, "ResearchModifiers batch", address, once);
            let old = batch_capture(&before, 1, &inputs, true);
            let new = batch_capture(&after, 1, &inputs, false);
            if once {
                never_harsher(old.0, new.0);
            } else {
                assert_eq!(new, old, "persistent {address:#x}");
            }
            assert!(
                after
                    .fallbacks
                    .witnesses()
                    .iter()
                    .filter(|&&r| r == "modifier_technologies")
                    .count()
                    <= 1
            );
            assert!(
                after
                    .fallbacks
                    .witnesses()
                    .iter()
                    .filter(|&&r| r == "modifier_ideas")
                    .count()
                    <= 2
            );
            assert!(
                after
                    .fallbacks
                    .witnesses()
                    .iter()
                    .filter(|&&r| r == "modifier_scripts")
                    .count()
                    <= 2
            );
        }
    }
}
