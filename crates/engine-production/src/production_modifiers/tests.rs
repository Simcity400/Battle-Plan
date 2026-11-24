use super::*;
use engine_native::ccountry::synthetic::World;
use engine_native_parts::{
    cequipment_type as ty,
    cmilitary_production_line,
    cnaval_production_line,
    crailway_gun_production_line,
    cship_refit_production_line,
};
use std::{cell::Cell, collections::BTreeMap};

struct Fixture {
    world: World,
    globals: BTreeMap<usize, Vec<u8>>,
    global_reads: Cell<usize>,
    fault: Option<usize>,
    once: bool,
    spent: Cell<bool>,
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
        let equipment = world.alloc(0x5c0);
        let factories = world.alloc(24);
        let bonuses = world.alloc(9 * 16);
        world.u64(variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        world.u64(
            variant + 8,
            (9 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
        world.u64(variant + cequipment_variant::DEFINITION, equipment as u64);
        world.u64(equipment, ty::VTABLE_RVA.0 as u64);
        world.u64(equipment + ty::ARCHETYPE, equipment as u64);
        for (index, value) in [-1_i64, 3276800, 123456].into_iter().enumerate() {
            world.u64(factories + index * 8, value as u64);
        }
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
            world.u64(status, cproduction_status::VTABLE_RVA.0 as u64);
            world.u64(status + cproduction_status::COUNTRY, country as u64);
            world.u64(status + 0x3c0, 50000);
            if active {
                let pointer = world.alloc(0x100);
                let roster = world.alloc(8);
                world.u64(roster, pointer as u64);
                world.array(status + cproduction_status::LINES, roster, 1);
                world.u64(pointer, cmilitary_production_line::VTABLE_RVA.0 as u64);
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
        globals.insert(DEFINE_RVAS[0].0, (-3_i32).to_le_bytes().to_vec());
        globals.insert(DEFINE_RVAS[1].0, 87_i32.to_le_bytes().to_vec());
        for (rva, value) in DEFINE_RVAS[2..]
            .iter()
            .zip([123_i64, 345, 100, 200, 300, 400])
        {
            globals.insert(rva.0, value.to_le_bytes().to_vec());
        }
        for (category, key) in (700_u32..707).enumerate() {
            globals.insert(
                BONUS_KEYS_RVA.0 + category * 0x28,
                key.to_le_bytes().to_vec(),
            );
        }
        for (rva, value) in [0x033e_1020, 0x033e_109c, 0x033e_1130, 0x033e_11b4]
            .into_iter()
            .zip([-7_i32, 12, 34, 56])
        {
            globals.insert(rva, value.to_le_bytes().to_vec());
        }
        globals.insert(0x033e_23e8, (-450_i64).to_le_bytes().to_vec());
        globals.insert(
            crate::dlc_feature_state::MASK_RVA.0,
            0_u32.to_le_bytes().to_vec(),
        );
        for (vtable, predicate) in [
            (cmilitary_production_line::VTABLE_RVA, 0x0011_d530_u64),
            (cnaval_production_line::VTABLE_RVA, 0x0011_d530),
            (crailway_gun_production_line::VTABLE_RVA, 0x0011_d530),
            (cship_refit_production_line::VTABLE_RVA, 0x0018_8d90),
        ] {
            globals.insert(vtable.0 + 0xd8, predicate.to_le_bytes().to_vec());
        }
        Self {
            world,
            globals,
            global_reads: Cell::new(0),
            fault: None,
            once: false,
            spent: Cell::new(false),
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
        }) && !(self.once && self.spent.replace(true))
        {
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
    let mut archetypes = vec![0; 2];
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
fn reader_acquires_authenticated_countries_and_their_ordered_retained_inputs() {
    let mut fixture = Fixture::new(2, true);
    fixture.fault = Some(fixture.bonuses + 8 * 16);
    let (result, sample) = acquire(&fixture, 2, 6);
    assert_eq!(result, Ok(()));
    assert_eq!(sample.required, [2, 2, 6, 2 * KEYS.len()]);
    assert_eq!(
        sample.context.unwrap().retention,
        [-7000, 12000, 34000, 56000]
    );
    assert_eq!(fixture.global_reads.get(), 21 + 2);
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
            fixture.world.object(country.country) + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR
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
fn header_demand_presizes_retained_factory_arrays() {
    let mut fixture = Fixture::new(2, true);
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
            countries: 2,
            lines: 2,
            efficiencies: 300,
            requests: 2 * KEYS.len(),
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
fn a_short_factory_arena_grows_and_a_short_native_read_refuses_the_supplement() {
    let mut fixture = Fixture::new(1, true);
    let (result, sample) = acquire(&fixture, 1, 2);
    assert_eq!(result, Ok(()));
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
        .u64(pointer, cnaval_production_line::VTABLE_RVA.0 as u64);
    let (result, sample) = acquire(&fixture, 1, 3);
    assert_eq!(result, Ok(()));
    assert!(sample.lines[0].naval_efficiency);
    assert!(!sample.lines[0].naval_output);
    assert!(!sample.lines[0].refit_output);
    fixture
        .world
        .u64(pointer, crailway_gun_production_line::VTABLE_RVA.0 as u64);
    let (result, sample) = acquire(&fixture, 1, 3);
    assert_eq!(result, Ok(()));
    assert_eq!(sample.lines[0].kind, line::KIND_RAILWAY_GUN);
    assert!(!sample.lines[0].refit_output);
    fixture
        .world
        .u64(pointer, cship_refit_production_line::VTABLE_RVA.0 as u64);
    fixture
        .world
        .u64(fixture.equipment + ty::PRIMARY_CATEGORY_FLAGS, 4);
    fixture.world.u32(fixture.equipment + ty::SLOTS_COUNT, 1);
    fixture.world.u64(fixture.variant + 0x3f8, 1);
    fixture.world.put(pointer + 0xec, &[1]);
    fixture.globals.insert(
        crate::dlc_feature_state::MASK_RVA.0,
        (1_u32 << 7).to_le_bytes().to_vec(),
    );
    let (result, sample) = acquire(&fixture, 1, 3);
    assert_eq!(result, Ok(()));
    assert!(sample.lines[0].refit_output);
    assert!(!sample.lines[0].conversion_output);
    fixture.globals.insert(
        crate::dlc_feature_state::MASK_RVA.0,
        0_u32.to_le_bytes().to_vec(),
    );
    assert!(acquire(&fixture, 1, 3).1.lines[0].conversion_output);
    fixture.globals.insert(
        cship_refit_production_line::VTABLE_RVA.0 + 0xd8,
        123_u64.to_le_bytes().to_vec(),
    );
    assert_eq!(
        acquire(&fixture, 1, 3).0,
        Err("unreached production refit virtual implementation")
    );
}

#[expect(
    clippy::type_complexity,
    reason = "the outcome, context, countries, lines and requests are returned together for comparison"
)]
fn growing(
    fixture: &Fixture,
    slots: std::ops::Range<u32>,
    provided: Option<Result<Context, &'static str>>,
) -> (
    Result<(), &'static str>,
    Option<Context>,
    Vec<Country>,
    Vec<(Line, Vec<i64>)>,
    Vec<Request>,
) {
    let (mut countries, mut lines, mut efficiencies, mut requests, mut archetypes) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut context = None;
    let mut out = Buffers {
        countries: &mut countries,
        country_len: 0,
        country_required: 0,
        lines: &mut lines,
        line_len: 0,
        line_required: 0,
        efficiencies: &mut efficiencies,
        efficiency_len: 0,
        efficiency_required: 0,
        requests: &mut requests,
        request_len: 0,
        request_required: 0,
        archetypes: &mut archetypes,
        archetype_required: 0,
    };
    let result = capture_slots(
        fixture,
        fixture.world.module,
        fixture.world.game_state(),
        slots,
        provided,
        &mut context,
        &mut out,
    );
    let lengths = [out.country_len, out.line_len, out.request_len];
    let lines = lines[..lengths[1]]
        .iter()
        .map(|line| {
            let [start, len] = line.efficiencies;
            (
                Line {
                    efficiencies: [0, 0],
                    ..*line
                },
                efficiencies[start..start + len].to_vec(),
            )
        })
        .collect();
    countries.truncate(lengths[0]);
    requests.truncate(lengths[2]);
    (result, context, countries, lines, requests)
}

#[test]
fn slot_parts_concatenate_to_the_whole_walk_and_meet_its_first_failure() {
    let mut fixture = Fixture::new(200, true);
    let bounds = [1, 37, 90, 151, u32::MAX];
    let parts = |fixture: &Fixture| {
        let provided = Context::read(fixture, fixture.world.module);
        bounds
            .windows(2)
            .map(|range| growing(fixture, range[0]..range[1], Some(provided)))
            .collect::<Vec<_>>()
    };
    let whole = growing(&fixture, 1..u32::MAX, None);
    assert_eq!(whole.0, Ok(()));
    assert_eq!(whole.2.len(), 200);
    let split = parts(&fixture);
    assert!(
        split
            .iter()
            .all(|part| part.0 == Ok(()) && part.1 == whole.1)
    );
    assert_eq!(
        split
            .iter()
            .flat_map(|part| part.2.clone())
            .collect::<Vec<_>>(),
        whole.2
    );
    assert_eq!(
        split
            .iter()
            .flat_map(|part| part.3.clone())
            .collect::<Vec<_>>(),
        whole.3
    );
    assert_eq!(
        split
            .iter()
            .flat_map(|part| part.4.clone())
            .collect::<Vec<_>>(),
        whole.4
    );
    fixture.fault = Some(fixture.lines[119]);
    let whole = growing(&fixture, 1..u32::MAX, None);
    assert!(whole.0.is_err());
    let split = parts(&fixture);
    assert_eq!(split[0].0, Ok(()));
    assert_eq!(split[1].0, Ok(()));
    assert_eq!(split[2].0, whole.0);
    fixture.fault = None;
    fixture.globals.clear();
    let whole = growing(&fixture, 1..u32::MAX, None);
    assert!(whole.0.is_err());
    assert_eq!(parts(&fixture)[0].0, whole.0);
}

#[test]
fn slot_parts_match_the_whole_walk_at_scale_and_under_every_fault() {
    let mut fixture = Fixture::new(5_000, true);
    let status = |fixture: &Fixture, slot: u32| {
        engine_native_memory::read::<u64>(
            &fixture.world.memory(),
            fixture.world.object(slot) + ccountry::PRODUCTION_STATUS,
        )
        .unwrap() as usize
    };
    for slot in 2_000..2_600 {
        let status = status(&fixture, slot);
        fixture
            .world
            .array(status + cproduction_status::LINES, 0, 0);
    }
    for slot in (7..=5_000)
        .step_by(7)
        .filter(|slot| !(2_000..2_600).contains(slot))
    {
        let roster = fixture.world.alloc(24);
        let line = fixture.lines[slot as usize - 1];
        for index in 0..3 {
            fixture.world.u64(roster + index * 8, line as u64);
        }
        let status = status(&fixture, slot);
        fixture
            .world
            .array(status + cproduction_status::LINES, roster, 3);
    }
    let bounds = [1, 1_200, 2_000, 2_600, 2_600, u32::MAX];
    let split = |fixture: &Fixture| {
        fixture.spent.set(false);
        let provided = Context::read(fixture, fixture.world.module);
        bounds
            .windows(2)
            .map(|range| growing(fixture, range[0]..range[1], Some(provided)))
            .collect::<Vec<_>>()
    };
    let whole = |fixture: &Fixture| {
        fixture.spent.set(false);
        growing(fixture, 1..u32::MAX, None)
    };
    let healthy = whole(&fixture);
    assert_eq!(healthy.0, Ok(()));
    assert_eq!(healthy.2.len(), 5_000 - 600);
    let parts = split(&fixture);
    assert!(parts[2].2.is_empty() && parts[3].2.is_empty());
    for part in &parts {
        assert_eq!(part.0, Ok(()));
        let expected = if part.2.is_empty() { None } else { healthy.1 };
        assert_eq!(part.1, expected);
    }
    assert_eq!(
        parts
            .iter()
            .flat_map(|part| part.2.clone())
            .collect::<Vec<_>>(),
        healthy.2
    );
    assert_eq!(
        parts
            .iter()
            .flat_map(|part| part.3.clone())
            .collect::<Vec<_>>(),
        healthy.3
    );
    assert_eq!(
        parts
            .iter()
            .flat_map(|part| part.4.clone())
            .collect::<Vec<_>>(),
        healthy.4
    );
    let part_of = |slot: u32| {
        bounds
            .windows(2)
            .position(|range| range[0] <= slot && slot < range[1])
            .unwrap()
    };
    let power = status(&fixture, 1_500) + 0x3c0;
    let line = fixture.lines[3_499] + cproduction_line::ACTIVE_FACTORIES;
    for (fault, slot) in [(power, 1_500), (line, 3_500), (DEFINE_RVAS[0].0, 1)] {
        for once in [false, true] {
            fixture.fault = Some(fault);
            fixture.once = once;
            let serial = whole(&fixture);
            let parts = split(&fixture);
            let at = part_of(slot);
            assert!(serial.0.is_err(), "{fault:x} once {once}");
            assert_eq!(parts[at].0, serial.0, "{fault:x} once {once}");
            for (index, part) in parts[..at].iter().enumerate() {
                assert_eq!(part.0, Ok(()), "{fault:x} once {once} part {index}");
            }
            let before: Vec<_> = parts[..at].iter().flat_map(|part| part.2.clone()).collect();
            assert_eq!(
                before[..],
                healthy.2[..before.len()],
                "{fault:x} once {once}"
            );
        }
    }
    fixture.fault = None;
}
