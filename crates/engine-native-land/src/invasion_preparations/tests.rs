use super::*;
use engine_native::cgame_state;
use engine_native_parts::modifier_definition;
use std::collections::BTreeMap;

#[derive(Default)]
struct Image(
    BTreeMap<usize, u8>,
    Option<Result<engine_native_memory::country_identity::Country, &'static str>>,
);

impl Memory for Image {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.0.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine_native_memory::country_identity::Country, &'static str>> {
        ((base, state) == (0, GS)).then_some(self.1).flatten()
    }
}

impl Image {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0
            .extend(bytes.iter().enumerate().map(|(i, b)| (at + i, *b)));
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn accumulator(&mut self, at: usize, data: usize, entries: &[(u32, i64)]) {
        self.array(at, data, entries.len() as u32);
        for (index, &(key, raw)) in entries.iter().enumerate() {
            self.u64(data + index * 16, u64::from(key));
            self.u64(data + index * 16 + 8, raw as u64);
        }
    }
}

const GS: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x2100;
const PROVINCE_TABLE: usize = 0x2200;
const COUNTRY: usize = 0x3000;
const ENEMY: usize = 0x3800;
const DIPLOMACY: usize = 0x4000;
const MATRIX: usize = 0x4100;
const RELATION: usize = 0x4800;
const GROUP: usize = 0x6000;
const PARENT: usize = 0x6800;
const ORDER: usize = 0x7000;
const OTHER: usize = 0x7800;
const LEADER: usize = 0x8000;
const MEMBER: usize = 0x9000;
const PROVINCE: usize = 0xa000;
const STATE: usize = 0xb000;
const DEFINITIONS: usize = 0x10_0000;
const HANDLE: u64 = (7 << 32) | 53;

fn fixture() -> Image {
    let mut m = Image::default();
    m.array(GS + cgame_state::COUNTRIES, COUNTRIES, 3);
    m.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 4);
    m.u64(COUNTRIES + 8, COUNTRY as u64);
    m.u64(COUNTRIES + 16, ENEMY as u64);
    m.u32(LINKS + 4, 1);
    m.u32(LINKS + 8, 1);
    m.u32(LINKS + 12, 2);
    for (country, id) in [(COUNTRY, 2), (ENEMY, 3)] {
        m.u64(country, ccountry::VTABLE_RVA.0 as u64);
        m.u32(country + ccountry::ID, id);
    }
    m.accumulator(
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x3400,
        &[(0x1a, -10_000), (0x184, 200_000)],
    );
    m.u64(COUNTRY + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
    m.u64(DIPLOMACY, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
    m.array(DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::RELATIONS, MATRIX, 3);
    m.put(MATRIX, &[0; 24]);
    m.u64(MATRIX + 16, RELATION as u64);
    m.u64(RELATION, crelation_status::VTABLE_RVA.0 as u64);
    m.accumulator(
        RELATION + RELATION_ACCUMULATOR,
        0x4c00,
        &[(0x1a, 30_000), (0x1ce, 40_000)],
    );
    m.array(GS + cgame_state::PROVINCES, PROVINCE_TABLE, 8);
    m.u64(PROVINCE_TABLE + 5 * 8, PROVINCE as u64);
    m.u64(PROVINCE, cprovince::VTABLE_RVA.0 as u64);
    m.u32(PROVINCE + cprovince::ID, 5);
    m.u64(PROVINCE + cprovince::STATE, STATE as u64);
    m.u64(STATE, cstate::VTABLE_RVA.0 as u64);
    m.u32(STATE + cstate::CONTROLLER, 3);
    m.u64(GROUP + corders_group::HANDLE, HANDLE);
    m.u32(GROUP + corders_group::COUNTRY, 2);
    m.u64(GROUP + corders_group::LEADER, LEADER as u64);
    m.u64(GROUP + corders_group::HQ_PROVIDER, MEMBER as u64);
    m.u64(GROUP + corders_group::PARENT, 0);
    m.array(GROUP + corders_group::PRIMARY_ORDER_DATA, 0x6400, 1);
    m.u64(0x6400, ORDER as u64);
    m.accumulator(LEADER + LEADER_ACCUMULATOR, 0x8800, &[(0x1a, 50_000)]);
    m.u64(MEMBER + corders_group_member::GROUP, GROUP as u64);
    m.u32(ORDER + corder_instance::TYPE, 3);
    m.u64(ORDER + corder_instance::OWNER, GROUP as u64);
    m.u32(ORDER + corder_instance::ID, 9);
    m.u64(ORDER + corder_instance::ACCUMULATED_PREPARATION, 1_234_567);
    m.array(ORDER + corder_instance::PATH_DATA, 0x7400, 2);
    m.u32(0x7400, 5);
    m.u32(0x7404, 6);
    m.array(ORDER + corder_instance::ASSIGNED_MEMBER_DATA, 0x7500, 2);
    m.u64(0x7500, 0x9999);
    m.u64(0x7508, MEMBER as u64);
    m.u64(modifier_definition::TABLE_RVA.0, DEFINITIONS as u64);
    for key in [0x1a_usize, 0x184, 0x1ce] {
        let row = DEFINITIONS + key * modifier_definition::STRIDE;
        m.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
        m.u32(row + modifier_definition::FLAGS, 0);
    }
    m
}

fn run(memory: &Image, order: usize) -> Result<Option<Row>, &'static str> {
    let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(16, 16).unwrap();
    country_memo.reset(0, GS);
    let countries = country_memo.view(memory, 0, GS);
    let provinces = invasion_provinces(memory, GS)?;
    read(
        memory,
        0,
        &countries,
        provinces,
        order,
        &DefinitionMemo::default(),
    )
}

#[test]
fn an_invasion_records_every_term_of_the_getter() {
    let row = run(&fixture(), ORDER).unwrap().unwrap();
    assert_eq!(
        row,
        Row {
            owner_group: HANDLE,
            instance: 9,
            progress: 1_234_567,
            target: 3,
            country_term: -10_000,
            relation_term: Some(30_000),
            relation_bonus: Some(40_000),
            leader_term: Some(50_000),
            leader_applies: 1,
            days_term: 200_000,
            ..Row::default()
        }
    );
    assert_eq!(row.key(), (u128::from(HANDLE) << 32) | 9);
}

#[test]
fn the_root_order_falls_back_to_the_parent_groups_orders() {
    let mut memory = fixture();
    memory.u64(0x6400, OTHER as u64);
    memory.array(OTHER + corder_instance::ASSIGNED_MEMBER_DATA, 0x7900, 0);
    memory.u64(GROUP + corders_group::PARENT, PARENT as u64);
    memory.array(PARENT + corders_group::PRIMARY_ORDER_DATA, 0x6c00, 1);
    memory.u64(0x6c00, ORDER as u64);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!((row.leader_term, row.leader_applies), (Some(50_000), 1));
    memory.u64(GROUP + corders_group::PARENT, 0);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!((row.leader_term, row.leader_applies), (Some(50_000), 0));
}

#[test]
fn an_earlier_order_assigned_the_provider_is_its_root() {
    let mut memory = fixture();
    memory.array(GROUP + corders_group::PRIMARY_ORDER_DATA, 0x6400, 2);
    memory.u64(0x6400, OTHER as u64);
    memory.u64(0x6408, ORDER as u64);
    memory.array(OTHER + corder_instance::ASSIGNED_MEMBER_DATA, 0x7900, 1);
    memory.u64(0x7900, MEMBER as u64);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(row.leader_applies, 0);
}

#[test]
fn no_leader_or_no_provider_has_no_leader_term() {
    for offset in [corders_group::LEADER, corders_group::HQ_PROVIDER] {
        let mut memory = fixture();
        memory.u64(GROUP + offset, 0);
        let row = run(&memory, ORDER).unwrap().unwrap();
        assert_eq!((row.leader_term, row.leader_applies), (None, 0));
    }
}

#[test]
fn an_empty_path_targets_zero_and_reads_slot_zero() {
    let mut memory = fixture();
    memory.array(ORDER + corder_instance::PATH_DATA, 0x7400, 0);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(
        (row.target, row.relation_term, row.relation_bonus),
        (0, None, None)
    );
    memory.u64(MATRIX, RELATION as u64);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(
        (row.target, row.relation_term, row.relation_bonus),
        (0, None, Some(40_000))
    );
}

#[test]
fn a_state_less_province_targets_zero() {
    let mut memory = fixture();
    memory.u64(PROVINCE + cprovince::STATE, 0);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(row.target, 0);
}

#[test]
fn the_same_country_takes_the_bonus_without_the_relation_term() {
    let mut memory = fixture();
    memory.u32(LINKS + 12, 1);
    memory.u64(MATRIX + 8, RELATION as u64);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(
        (row.target, row.relation_term, row.relation_bonus),
        (3, None, Some(40_000))
    );
    memory.u64(MATRIX + 8, 0);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!((row.relation_term, row.relation_bonus), (None, None));
}

#[test]
fn a_path_province_outside_the_table_is_refused() {
    let mut memory = fixture();
    memory.u32(0x7400, 8);
    assert!(run(&memory, ORDER).is_err());
    memory.u32(0x7400, 0);
    assert!(run(&memory, ORDER).is_err());
}

#[test]
fn other_order_types_have_no_row() {
    for kind in [
        corder_instance::TYPE_MOVE,
        corder_instance::TYPE_LINE,
        corder_instance::TYPE_PARADROP,
        corder_instance::TYPE_AREA_DEFENSE,
    ] {
        let mut memory = fixture();
        memory.u32(ORDER + corder_instance::TYPE, kind as u32);
        assert_eq!(run(&memory, ORDER), Ok(None));
    }
}

fn visit(pointer: usize) -> corder_instance::Visit {
    let mut visit = corder_instance::Visit::default();
    visit.pointer = pointer;
    visit
}

const UNIT: usize = 0x9999 - cunit::SCHEDULED_MEMBER;
const ZERO: usize = 0x4400;

fn route_fixture() -> Image {
    let mut m = fixture();
    m.u32(UNIT + cunit::LOGICAL_OWNER, 2);
    m.u64(DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY, COUNTRY as u64);
    m.u64(MATRIX, ZERO as u64);
    for (status, target) in [(ZERO, 0_u32), (RELATION, 3)] {
        m.u64(status, crelation_status::VTABLE_RVA.0 as u64);
        m.u64(
            status + crelation_status::DIPLOMACY_STATUS,
            DIPLOMACY as u64,
        );
        m.u32(status + crelation_status::TARGET_COUNTRY, target);
        m.u64(status + crelation_status::WAR_RELATION, 0);
        m.put(status + crelation_status::CO_BELLIGERENT, &[1]);
    }
    m.array(DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::WARTIME_ALLIES, 0x4f00, 1);
    m.u32(0x4f00, 3);
    m.array(DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES, 0x4f80, 0);
    m
}

#[test]
fn the_walk_records_invasions_counts_overflow_and_needs_the_census() {
    let mut memory = route_fixture();
    memory.u32(OTHER + corder_instance::TYPE, 1);
    let visits = [visit(ORDER), visit(0), visit(OTHER)];
    let memo = DefinitionMemo::default();
    let mut rows = vec![Row::default(); 1];
    let source = Outcome::Rows { len: 2 };
    let walked = |memory: &Image, gs, rows: &mut Vec<Row>, source| {
        let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(16, 16).unwrap();
        country_memo.reset(0, gs);
        let countries = country_memo.view(memory, 0, gs);
        walk(memory, 0, gs, &visits, &[], source, &memo, &countries, rows)
    };
    assert_eq!(
        walked(&memory, GS, &mut rows, source),
        Walk {
            outcome: Outcome::Rows { len: 1 }
        }
    );
    assert_eq!((rows[0].instance, rows[0].unit_owner), (9, 2));
    let mut unreadable = route_fixture();
    unreadable.u32(OTHER + corder_instance::TYPE, 1);
    unreadable.u64(MATRIX, 0);
    assert_eq!(
        walked(&unreadable, GS, &mut rows, source).outcome,
        Outcome::Rows { len: 1 }
    );
    memory.u32(ORDER + corder_instance::TYPE, 2);
    assert_eq!(
        walked(&memory, 0x50_0000, &mut rows, source).outcome,
        Outcome::Rows { len: 0 }
    );
    for source in [
        Outcome::Rows { len: 3 },
        Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        },
    ] {
        assert!(matches!(
            walked(&memory, GS, &mut rows, source).outcome,
            Outcome::Unavailable { .. }
        ));
    }
}

#[test]
fn invasion_rows_are_acquired_in_census_order() {
    let mut memory = route_fixture();
    let original: Vec<_> = memory
        .0
        .range(ORDER..ORDER + 0x400)
        .map(|(&at, &byte)| (at - ORDER, byte))
        .collect();
    let pointers = [0x200_0000, 0x200_0400];
    for (index, pointer) in pointers.into_iter().enumerate() {
        for &(offset, byte) in &original {
            memory.0.insert(pointer + offset, byte);
        }
        memory.u32(pointer + corder_instance::ID, index as u32 + 1);
    }
    let table = [visit(pointers[1]), visit(pointers[0])];
    let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(16, 16).unwrap();
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    let mut rows = Vec::new();
    let report = walk(
        &memory,
        0,
        GS,
        &table,
        &[1, 0],
        Outcome::Rows { len: 2 },
        &DefinitionMemo::default(),
        &countries,
        &mut rows,
    );
    assert_eq!(report.outcome, Outcome::Rows { len: 2 });
    assert_eq!([rows[0].instance, rows[1].instance], [1, 2]);
}

#[test]
fn invasion_rows_grow_before_first_and_later_publication() {
    let mut rows = Vec::new();
    for count in [50, 75] {
        let mut memory = route_fixture();
        let original: Vec<_> = memory
            .0
            .range(ORDER..ORDER + 0x400)
            .map(|(&at, &byte)| (at - ORDER, byte))
            .collect();
        let visits: Vec<_> = (0..count)
            .map(|index| {
                let pointer = 0x200_0000 + index * 0x400;
                for &(offset, byte) in &original {
                    memory.0.insert(pointer + offset, byte);
                }
                memory.u32(pointer + corder_instance::ID, index as u32 + 1);
                visit(pointer)
            })
            .collect();
        let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(16, 16).unwrap();
        country_memo.reset(0, GS);
        let countries = country_memo.view(&memory, 0, GS);
        let report = walk(
            &memory,
            0,
            GS,
            &visits,
            &[],
            Outcome::Rows { len: count },
            &DefinitionMemo::default(),
            &countries,
            &mut rows,
        );
        assert_eq!(report.outcome, Outcome::Rows { len: count });
        assert_eq!(rows[count - 1].instance, count as u32);
        assert!(rows[..count].iter().all(|row| row.unit_owner == 2));
    }
}

#[test]
fn the_route_unit_falls_back_to_the_group_then_the_request_countrys_roster() {
    let mut memory = route_fixture();
    let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(16, 16).unwrap();
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    assert_eq!(route_unit(&memory, &countries, ORDER, GROUP), Ok(UNIT));
    memory.array(ORDER + corder_instance::ASSIGNED_MEMBER_DATA, 0x7500, 0);
    memory.array(GROUP + corders_group::MEMBER_DATA, 0x6600, 1);
    memory.u64(0x6600, MEMBER as u64);
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    assert_eq!(
        route_unit(&memory, &countries, ORDER, GROUP),
        Ok(MEMBER - cunit::SCHEDULED_MEMBER)
    );
    memory.array(GROUP + corders_group::MEMBER_DATA, 0x6600, 0);
    memory.u64(GROUP + corders_group::THEATRE, 0x6a00);
    memory.u32(0x6a00 + ctheatre::VOLUNTEERS_THEATRE, 0);
    memory.u32(0x6a00 + ctheatre::OWNER, 2);
    memory.array(COUNTRY + ccountry::DIVISIONS, 0x6b00, 2);
    memory.u64(0x6b00, 0x5500);
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    assert_eq!(route_unit(&memory, &countries, ORDER, GROUP), Ok(0x5500));
    memory.u32(0x6a00 + ctheatre::VOLUNTEERS_THEATRE, 2);
    memory.u32(0x6a00 + ctheatre::OWNER, 99);
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    assert_eq!(route_unit(&memory, &countries, ORDER, GROUP), Ok(0x5500));
    memory.array(COUNTRY + ccountry::DIVISIONS, 0x6b00, 0);
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    assert_eq!(route_unit(&memory, &countries, ORDER, GROUP), Ok(0));
}

fn reserved(memory: &mut Image) {
    memory.1 = Some(Ok(engine_native_memory::country_identity::Country {
        pointer: COUNTRY,
        canonical_slot: 0,
        id: 0,
    }));
    memory.u32(COUNTRY + ccountry::ID, 0);
    memory.u32(GROUP + corders_group::COUNTRY, 0);
    memory.u64(MATRIX, RELATION as u64);
}

#[test]
fn raw_zero_preparation_and_logical_route_owner_need_no_positive_context() {
    let mut memory = route_fixture();
    reserved(&mut memory);
    memory.u32(STATE + cstate::CONTROLLER, 0);
    memory.u32(UNIT + cunit::LOGICAL_OWNER, 0);
    memory.u32(RELATION + crelation_status::TARGET_COUNTRY, 0);
    memory.u32(0x4f00, 0);
    for header in [cgame_state::COUNTRIES, cgame_state::COUNTRY_LINKS] {
        for byte in 0..16 {
            memory.0.remove(&(GS + header + byte));
        }
    }
    let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
    country_memo.reset(0, GS);
    let countries = country_memo.view(&memory, 0, GS);
    let mut rows = vec![Row::default(); 1];
    let walked = walk(
        &memory,
        0,
        GS,
        &[visit(ORDER)],
        &[],
        Outcome::Rows { len: 1 },
        &DefinitionMemo::default(),
        &countries,
        &mut rows,
    );
    assert_eq!(walked.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].country_term, -10_000);
    assert_eq!(rows[0].relation_term, None);
    assert_eq!(rows[0].relation_bonus, Some(40_000));
    assert_eq!(rows[0].unit_owner, 0);
    assert_eq!(country_memo.link_required(), 0);
}

#[test]
fn native_country_id_zero_preserves_positive_alias_zero_relation_gate() {
    let mut memory = fixture();
    reserved(&mut memory);
    memory.u32(LINKS + 3 * 4, 0);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(row.target, 3);
    assert_eq!(row.relation_term, Some(30_000));
    assert_eq!(row.relation_bonus, Some(40_000));
}

#[test]
fn preparation_group_readable_alias_zero_uses_the_same_reserved_source() {
    let mut memory = fixture();
    reserved(&mut memory);
    memory.u32(GROUP + corders_group::COUNTRY, 2);
    memory.u32(LINKS + 2 * 4, 0);
    memory.u32(STATE + cstate::CONTROLLER, 0);
    let row = run(&memory, ORDER).unwrap().unwrap();
    assert_eq!(row.country_term, -10_000);
    assert_eq!(row.days_term, 200_000);
    assert_eq!(row.relation_term, None);
    assert_eq!(row.relation_bonus, Some(40_000));
}

#[test]
fn theatre_fallback_uses_native_zero_and_readable_alias_zero() {
    let mut memory = fixture();
    reserved(&mut memory);
    memory.array(ORDER + corder_instance::ASSIGNED_MEMBER_DATA, 0x7500, 0);
    memory.array(GROUP + corders_group::MEMBER_DATA, 0x6600, 0);
    memory.u64(GROUP + corders_group::THEATRE, 0x6a00);
    memory.u32(0x6a00 + ctheatre::VOLUNTEERS_THEATRE, 0);
    memory.u32(0x6a00 + ctheatre::OWNER, 0);
    memory.array(COUNTRY + ccountry::DIVISIONS, 0x6b00, 1);
    memory.u64(0x6b00, UNIT as u64);
    let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(16, 16).unwrap();
    country_memo.reset(0, GS);
    assert_eq!(
        route_unit(&memory, &country_memo.view(&memory, 0, GS), ORDER, GROUP),
        Ok(UNIT)
    );
    assert_eq!(country_memo.link_required(), 0);
    memory.u32(0x6a00 + ctheatre::VOLUNTEERS_THEATRE, 2);
    memory.u32(0x6a00 + ctheatre::OWNER, 99);
    memory.u32(LINKS + 2 * 4, 0);
    country_memo.reset(0, GS);
    assert_eq!(
        route_unit(&memory, &country_memo.view(&memory, 0, GS), ORDER, GROUP),
        Ok(UNIT)
    );
}
