use super::*;
use crate::{
    ccountry::synthetic::World, cequipment_type as ty, cmilitary_production_line,
    cnaval_production_line, crailway_gun_production_line, cship_refit_production_line,
};
use std::{cell::Cell, collections::BTreeMap};

struct Fixture {
    world: World,
    globals: BTreeMap<usize, Vec<u8>>,
    global_reads: Cell<usize>,
    fault: Option<usize>,
    lines: Vec<usize>,
    variant: usize,
    equipment: usize,
    factories: usize,
    bonuses: usize,
}
impl Fixture {
    fn new(count: u32, active: bool) -> Self {
        let mut world = World::new(0, count, ccountry::PRODUCTION_STATUS + 8);
        let variant = world.alloc(0x420);
        let equipment = world.alloc(0x560);
        let factories = world.alloc(24);
        let bonuses = world.alloc(9 * 16);
        world.u64(variant, cequipment_variant::VTABLE_RVA as u64);
        world.u64(
            variant + 8,
            (9 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
        world.u64(variant + cequipment_variant::DEFINITION, equipment as u64);
        world.u64(equipment, ty::VTABLE_RVA as u64);
        world.u64(equipment + ty::ARCHETYPE, equipment as u64);
        for (index, value) in [-1_i64, 3276800, 123456].into_iter().enumerate() {
            world.u64(factories + index * 8, value as u64);
        }
        // First equal category-3 value wins; the unreadable tail is unnecessary
        // once every requested runtime token has its first match.
        for (index, (category, value)) in [
            (3, -7_i64),
            (3, 999),
            (0, 11),
            (1, 12),
            (2, 13),
            (4, 14),
            (5, 15),
            (6, 16),
        ]
        .into_iter()
        .enumerate()
        {
            world.u32(bonuses + index * 16, 700 + category);
            world.u64(bonuses + index * 16 + 8, value as u64);
        }
        let mut lines = Vec::new();
        for slot in 1..=count {
            let country = world.object(slot);
            let status = world.alloc(0x400);
            world.u64(country + ccountry::PRODUCTION_STATUS, status as u64);
            world.u64(status, cproduction_status::VTABLE_RVA as u64);
            world.u64(status + cproduction_status::COUNTRY, country as u64);
            world.u64(status + 0x3a8, 50000);
            if active {
                let pointer = world.alloc(0x100);
                let roster = world.alloc(8);
                world.u64(roster, pointer as u64);
                world.array(status + cproduction_status::LINES, roster, 1);
                world.u64(pointer, cmilitary_production_line::VTABLE_RVA as u64);
                world.u64(pointer + 8, (u64::from(slot) << 32) | 56);
                world.u64(pointer + cproduction_line::PRODUCTION_STATUS, status as u64);
                world.u32(pointer + cproduction_line::ACTIVE_FACTORIES, 1);
                world.u64(pointer + line::EQUIPMENT_VARIANT, variant as u64);
                world.u64(
                    pointer + line::INDUSTRIAL_ORGANISATION_CID,
                    (u64::from(slot) << 32) | 127,
                );
                world.array(pointer + 0xa0, bonuses, 9);
                world.put(pointer + 0xb8, &[1]);
                world.array(pointer + line::FACTORY_EFFICIENCIES, factories, 3);
                world.u64(pointer + 0xd8, -25000_i64 as u64);
                lines.push(pointer);
            }
        }
        let mut globals = BTreeMap::new();
        globals.insert(DEFINE_RVAS[0], (-3_i32).to_le_bytes().to_vec());
        globals.insert(DEFINE_RVAS[1], 87_i32.to_le_bytes().to_vec());
        for (rva, value) in DEFINE_RVAS[2..]
            .iter()
            .zip([123_i64, 345, 100, 200, 300, 400])
        {
            globals.insert(*rva, value.to_le_bytes().to_vec());
        }
        for (category, key) in (700_u32..707).enumerate() {
            globals.insert(BONUS_KEYS_RVA + category * 0x28, key.to_le_bytes().to_vec());
        }
        for (rva, value) in [0x0331_dcec, 0x0331_dd6c, 0x0331_de00, 0x0331_de9c]
            .into_iter()
            .zip([-7_i32, 12, 34, 56])
        {
            globals.insert(rva, value.to_le_bytes().to_vec());
        }
        globals.insert(0x0331_f158, (-450_i64).to_le_bytes().to_vec());
        globals.insert(
            crate::dlc_feature_state::MASK_RVA,
            0_u32.to_le_bytes().to_vec(),
        );
        for (vtable, predicate) in [
            (cmilitary_production_line::VTABLE_RVA, 0x0011_cd10_u64),
            (cnaval_production_line::VTABLE_RVA, 0x0011_cd10),
            (crailway_gun_production_line::VTABLE_RVA, 0x0011_cd10),
            (cship_refit_production_line::VTABLE_RVA, 0x0018_02a0),
        ] {
            globals.insert(vtable + 0xd8, predicate.to_le_bytes().to_vec());
        }
        Self {
            world,
            globals,
            global_reads: Cell::new(0),
            fault: None,
            lines,
            variant,
            equipment,
            factories,
            bonuses,
        }
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.fault.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) {
            return false;
        }
        if let Some(bytes) = self.globals.get(&address) {
            self.global_reads.set(self.global_reads.get() + 1);
            if bytes.len() < destination.len() {
                return false;
            }
            destination.copy_from_slice(&bytes[..destination.len()]);
            true
        } else {
            self.world.memory().copy(address, destination)
        }
    }
}
struct Sample {
    context: Option<Context>,
    countries: Vec<Country>,
    lines: Vec<Line>,
    efficiencies: Vec<i64>,
    requests: Vec<Request>,
    required: [usize; 4],
}
fn acquire(
    fixture: &Fixture,
    count: usize,
    efficiency_capacity: usize,
) -> (Result<(), &'static str>, Sample) {
    let mut sample = Sample {
        context: None,
        countries: vec![Country::default(); count],
        lines: vec![Line::default(); count],
        efficiencies: vec![0; efficiency_capacity],
        requests: vec![Request::default(); count * KEYS.len()],
        required: [0; 4],
    };
    let mut archetypes = [0; 2];
    let mut buffers = Buffers {
        countries: &mut sample.countries,
        country_len: 0,
        country_required: 0,
        lines: &mut sample.lines,
        line_len: 0,
        line_required: 0,
        efficiencies: &mut sample.efficiencies,
        efficiency_len: 0,
        efficiency_required: 0,
        requests: &mut sample.requests,
        request_len: 0,
        request_required: 0,
        archetypes: &mut archetypes,
        archetype_required: 0,
    };
    let result = capture(
        fixture,
        fixture.world.module,
        fixture.world.game_state(),
        &mut sample.context,
        &mut buffers,
    );
    sample.required = [
        buffers.country_required,
        buffers.line_required,
        buffers.efficiency_required,
        buffers.request_required,
    ];
    let lengths = [
        buffers.country_len,
        buffers.line_len,
        buffers.efficiency_len,
        buffers.request_len,
    ];
    sample.countries.truncate(lengths[0]);
    sample.lines.truncate(lengths[1]);
    sample.efficiencies.truncate(lengths[2]);
    sample.requests.truncate(lengths[3]);
    (result, sample)
}

#[test]
fn reader_acquires_five_thousand_authenticated_countries_and_their_ordered_retained_inputs() {
    let mut fixture = Fixture::new(5000, true);
    fixture.fault = Some(fixture.bonuses + 8 * 16);
    let (result, sample) = acquire(&fixture, 5000, 15000);
    assert_eq!(result, Ok(()));
    assert_eq!(sample.required, [5000, 5000, 15000, 55000]);
    assert_eq!(
        sample.context.unwrap().retention,
        [-7000, 12000, 34000, 56000]
    );
    assert_eq!(
        fixture.global_reads.get(),
        21 + 5000,
        "one context read, then one virtual predicate per line"
    );
    for (index, (country, line)) in sample.countries.iter().zip(&sample.lines).enumerate() {
        assert_eq!(country.country, index as u32 + 1);
        assert_eq!(line.country, country.country);
        assert_eq!(line.resource_penalty, -25000);
        assert!(line.organisation_dirty);
        assert_eq!(line.bonuses, [11, 12, 13, -7, 14, 15, 16]);
        assert_eq!(
            &sample.efficiencies[line.efficiencies[0]..line.efficiencies[0] + line.efficiencies[1]],
            [-1, 3276800, 123456]
        );
        assert_eq!(
            sample.requests[index * KEYS.len()].accumulator,
            fixture.world.object(country.country) + ccountry::MODIFIER_ACCUMULATOR
        );
        assert_eq!(
            sample.requests[index * KEYS.len()..(index + 1) * KEYS.len()]
                .iter()
                .map(|request| request.key)
                .collect::<Vec<_>>(),
            KEYS
        );
    }
}
#[test]
fn idle_rosters_do_not_require_global_defines_or_hover_operands() {
    let mut fixture = Fixture::new(2, false);
    fixture.globals.clear();
    let (result, sample) = acquire(&fixture, 2, 0);
    assert_eq!(result, Ok(()));
    assert_eq!(sample.context, None);
    assert_eq!(sample.required, [0; 4]);
    assert_eq!(fixture.global_reads.get(), 0);
}

#[test]
fn header_demand_presizes_retained_factory_arrays_for_five_thousand_countries() {
    let mut fixture = Fixture::new(5_000, true);
    for &pointer in &fixture.lines {
        fixture
            .world
            .array(pointer + line::FACTORY_EFFICIENCIES, fixture.factories, 150);
    }
    fixture.fault = Some(fixture.factories);
    let observed = demand(&fixture, fixture.world.module, fixture.world.game_state()).unwrap();
    assert_eq!(
        observed,
        Demand {
            countries: 5_000,
            lines: 5_000,
            efficiencies: 750_000,
            requests: 5_000 * KEYS.len(),
        }
    );
    assert_eq!(fixture.global_reads.get(), 0);
    fixture.fault = Some(fixture.lines[0] + line::FACTORY_EFFICIENCIES);
    assert_eq!(
        demand(&fixture, fixture.world.module, fixture.world.game_state()),
        Err("production modifier factory efficiencies unreadable or invalid")
    );
}
#[test]
fn short_factory_arena_retains_demand_and_short_native_read_refuses_the_supplement() {
    let mut fixture = Fixture::new(1, true);
    let (result, sample) = acquire(&fixture, 1, 2);
    assert_eq!(
        result,
        Err("production modifier caller arena capacity exceeded")
    );
    assert_eq!(sample.required, [1, 1, 3, KEYS.len()]);
    fixture.fault = Some(fixture.factories + 8);
    assert_eq!(
        acquire(&fixture, 1, 3).0,
        Err("production modifier factory efficiency unreadable")
    );
}
#[test]
fn reader_keeps_type_classifier_variant_flags_dlc_conversion_and_virtual_refit_separate() {
    let mut fixture = Fixture::new(1, true);
    let pointer = fixture.lines[0];
    fixture
        .world
        .u64(fixture.equipment + ty::PRIMARY_CATEGORY_FLAGS, 0x40);
    fixture
        .world
        .u64(pointer, cnaval_production_line::VTABLE_RVA as u64);
    let (result, sample) = acquire(&fixture, 1, 3);
    assert_eq!(result, Ok(()));
    assert!(sample.lines[0].naval_efficiency);
    assert!(!sample.lines[0].naval_output);
    assert!(!sample.lines[0].refit_output);
    fixture
        .world
        .u64(pointer, crailway_gun_production_line::VTABLE_RVA as u64);
    let (result, sample) = acquire(&fixture, 1, 3);
    assert_eq!(result, Ok(()));
    assert_eq!(sample.lines[0].kind, line::KIND_RAILWAY_GUN);
    assert!(!sample.lines[0].refit_output);
    fixture
        .world
        .u64(pointer, cship_refit_production_line::VTABLE_RVA as u64);
    fixture
        .world
        .u64(fixture.equipment + ty::PRIMARY_CATEGORY_FLAGS, 4);
    fixture.world.u32(fixture.equipment + ty::SLOTS_COUNT, 1);
    fixture.world.u64(fixture.variant + 0x3f8, 1);
    fixture.world.put(pointer + 0xec, &[1]);
    fixture.globals.insert(
        crate::dlc_feature_state::MASK_RVA,
        (1_u32 << 7).to_le_bytes().to_vec(),
    );
    let (result, sample) = acquire(&fixture, 1, 3);
    assert_eq!(result, Ok(()));
    assert!(sample.lines[0].refit_output);
    assert!(!sample.lines[0].conversion_output);
    fixture.globals.insert(
        crate::dlc_feature_state::MASK_RVA,
        0_u32.to_le_bytes().to_vec(),
    );
    assert!(acquire(&fixture, 1, 3).1.lines[0].conversion_output);
    fixture.globals.insert(
        cship_refit_production_line::VTABLE_RVA + 0xd8,
        123_u64.to_le_bytes().to_vec(),
    );
    assert_eq!(
        acquire(&fixture, 1, 3).0,
        Err("unreached production refit virtual implementation")
    );
}
