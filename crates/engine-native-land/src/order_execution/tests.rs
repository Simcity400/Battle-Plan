use super::*;
use std::collections::BTreeMap;

#[test]
fn retained_unit_ordinals_revalidate_current_order_and_alias_refusals() {
    let mut prepared = Prepared::try_new(capacity()).unwrap();
    let mut seeds: Vec<_> = [(30, 3), (10, 1), (20, 2)]
        .into_iter()
        .map(|(pointer, division)| carmy::DivisionSeed {
            pointer,
            division,
            ..Default::default()
        })
        .collect();
    for hour in 0..6 {
        match hour {
            1 => seeds.rotate_left(1),
            2 => seeds[1].pointer = seeds[0].pointer,
            3 => seeds[2].division = seeds[0].division,
            4 => {
                seeds.pop();
            }
            5 => seeds.clear(),
            _ => {}
        }
        prepared.reset();
        prepared.index_units(&seeds);
        let mut fresh = Prepared::try_new(capacity()).unwrap();
        fresh.index_units(&seeds);
        assert_eq!(prepared.pointers, fresh.pointers);
        assert_eq!(prepared.handles, fresh.handles);
        assert_eq!(prepared.invalid, fresh.invalid);
        for seed in &seeds {
            assert_eq!(
                prepared.host_index(&seeds, seed.pointer),
                fresh.host_index(&seeds, seed.pointer)
            );
            assert_eq!(
                prepared.unit_index(&seeds, seed.division, seed.pointer),
                fresh.unit_index(&seeds, seed.division, seed.pointer)
            );
        }
    }
}

const BASE: usize = 0x1000;
const STATE: usize = 0x2000;
const GROUP: usize = 0x3000;
const NAVAL: usize = 0x4000;
const PARA: usize = 0x5000;
const ARMY: usize = 0x6000;
const STAT: usize = 0x7000;
const COUNTRY: usize = 0x8000;
const PROVINCE: usize = 0x9000;
const DEFINITION: usize = 0xa000;
const HANDLE: u64 = (7_u64 << 32) | 53;
const UNIT: u64 = (3_u64 << 32) | 14;

#[test]
fn primary_memberships_are_prepared_only_for_the_consuming_paradrop_branch() {
    let image = Image::default();
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [order::Visit::fixture(PARA, 0, 1)];
    let groups = [corders_group::PlanRow {
        group: HANDLE,
        primary_len: 1,
        ..Default::default()
    }];
    let hosts = AssignedSources::fixture(vec![]);
    let mut prepared = Prepared::try_new(capacity()).unwrap();
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
    memo.reset(BASE, STATE);
    for (kind, target, route, demanded) in [
        (order::TYPE_INVASION, 1, Some(1), false),
        (order::TYPE_PARADROP, 0, Some(1), false),
        (order::TYPE_PARADROP, u32::MAX, Some(1), false),
        (order::TYPE_PARADROP, 1, None, false),
        (order::TYPE_PARADROP, 1, Some(0), false),
        (order::TYPE_PARADROP, 1, Some(1), true),
        (order::TYPE_INVASION, 1, Some(1), false),
    ] {
        let rows = [order::Row {
            owner_group: HANDLE,
            instance: 1,
            order_type: kind,
            paradrop_target: target,
            ..Default::default()
        }];
        let plans = [PlanRow {
            owner_group: HANDLE,
            instance: 1,
            execution: Some(OrderExecutionInputs {
                route_is_ok: route,
                ..Default::default()
            }),
            ..Default::default()
        }];
        let o = orders(&roots, &visits, &rows);
        prepared.reset();
        prepared.base = BASE;
        prepared.state = STATE;
        prepared.bound = true;
        prepared.index_orders(o);
        prepared.capture_units(
            &image,
            BASE,
            STATE,
            &[],
            Outcome::Rows { len: 0 },
            o,
            source(&plans, &groups, &[], &[], &hosts),
            &memo.view(&image, BASE, STATE),
        );
        assert_eq!(prepared.units.outcome(), Outcome::Rows { len: 0 });
        assert_eq!(
            prepared.membership_error,
            demanded.then_some("execution primary span incomplete")
        );
        assert_eq!(prepared.required.memberships, 0);
    }
    assert!(image.reads.borrow().is_empty());
}

#[derive(Default)]
struct Image {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    reserved: Option<Result<engine_native_memory::country_identity::Country, &'static str>>,
}
impl Image {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, &byte)| (address + index, byte)),
        );
    }
    fn word(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
}
impl Memory for Image {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + index)) else {
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
        ((base, state) == (BASE, STATE))
            .then_some(self.reserved)
            .flatten()
    }
}
fn capacity() -> Capacity {
    Capacity {
        units: 4,
        groups: 4,
        orders: 4,
        memberships: 8,
        scalar_observations: 64,
        modifiers: SnapshotCapacity {
            nodes: 4,
            entries: 4,
        },
    }
}
fn seed() -> carmy::DivisionSeed {
    carmy::DivisionSeed {
        pointer: ARMY,
        division: UNIT,
        handle: Some(UNIT),
        primary_vtable: Some((BASE + carmy::VTABLE_RVA.0) as u64),
        secondary_vtable: Some((BASE + carmy::SECONDARY_VTABLE_RVA.0) as u64),
        class: Some(0),
        orders_group: Some(GROUP),
        stat_block: Some(STAT),
        roster_owner: Some(0),
        location: Some(PROVINCE),
        naval_transfer_membership: Some(0),
        ..Default::default()
    }
}
fn orders<'a>(
    roots: &'a [corders_group::Root],
    visits: &'a [order::Visit],
    rows: &'a [order::Row],
) -> Orders<'a> {
    Orders {
        roots,
        root_source: Outcome::Rows { len: roots.len() },
        visits,
        rows,
        source: Outcome::Rows { len: rows.len() },
        geometry: &[],
        plan_rows: &[],
        census: &[],
    }
}
fn source<'a>(
    plans: &'a [PlanRow],
    groups: &'a [corders_group::PlanRow],
    primary: &'a [u32],
    assigned: &'a [u64],
    hosts: &'a AssignedSources,
) -> SourceView<'a> {
    SourceView {
        plans,
        source: Outcome::Rows { len: plans.len() },
        groups,
        group_source: Outcome::Rows { len: groups.len() },
        primary,
        assigned,
        assigned_source: Outcome::Rows {
            len: assigned.len(),
        },
        hosts,
    }
}

#[test]
fn route_zero_does_not_reach_progress_country_or_province() {
    let mut image = Image::default();
    image.put(PARA + order::CONDITION_CACHE, &[0]);
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [order::Visit::fixture(PARA, 0, 2)];
    let rows = [order::Row {
        owner_group: HANDLE,
        instance: 2,
        order_type: order::TYPE_PARADROP,
        path_start: usize::MAX,
        path_len: 1,
        ..Default::default()
    }];
    let mut plans = [PlanRow {
        owner_group: HANDLE,
        instance: 2,
        ..Default::default()
    }];
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
    memo.reset(BASE, STATE);
    let countries = memo.view(&image, BASE, STATE);
    let mut packet = Prepared::try_new(capacity()).unwrap();
    packet.capture_orders(
        &image,
        BASE,
        STATE,
        orders(&roots, &visits, &rows),
        &mut plans,
        Outcome::Rows { len: 1 },
        &countries,
    );
    assert_eq!(
        plans[0].execution,
        Some(OrderExecutionInputs {
            route_is_ok: Some(0),
            paradrop: None
        })
    );
    assert_eq!(
        &*image.reads.borrow(),
        &[(PARA + order::CONDITION_CACHE, 1)]
    );
    assert_eq!(packet.work().progress_demand, 0);
}

#[test]
fn any_plan_placement_gives_the_searched_execution() {
    const LINE: usize = 0xb000;
    let run = |plan_rows: &[u32]| {
        let mut image = Image::default();
        image.put(PARA + order::CONDITION_CACHE, &[0]);
        let roots = [corders_group::Root {
            pointer: GROUP,
            handle: HANDLE,
            country: 0,
        }];
        let visits = [
            order::Visit::fixture(PARA, 0, 2),
            order::Visit::fixture(LINE, 0, 1),
        ];
        let rows = [1, 2].map(|instance| order::Row {
            owner_group: HANDLE,
            instance,
            order_type: if instance == 2 {
                order::TYPE_PARADROP
            } else {
                order::TYPE_LINE
            },
            path_start: usize::MAX,
            path_len: 1,
            ..Default::default()
        });
        let mut plans = [1, 2].map(|instance| PlanRow {
            owner_group: HANDLE,
            instance,
            ..Default::default()
        });
        let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
        memo.reset(BASE, STATE);
        let countries = memo.view(&image, BASE, STATE);
        let mut packet = Prepared::try_new(capacity()).unwrap();
        packet.capture_orders(
            &image,
            BASE,
            STATE,
            Orders {
                plan_rows,
                ..orders(&roots, &visits, &rows)
            },
            &mut plans,
            Outcome::Rows { len: 2 },
            &countries,
        );
        let reads = image.reads.borrow().clone();
        (plans, reads)
    };
    let searched = run(&[]);
    assert_eq!(
        searched.0[1].execution,
        Some(OrderExecutionInputs {
            route_is_ok: Some(0),
            paradrop: None
        })
    );
    assert_eq!(searched.0[0].execution, None);
    for plan_rows in [&[1, 0][..], &[0, 1], &[1], &[7, 7]] {
        assert_eq!(run(plan_rows), searched, "{plan_rows:?}");
    }
}

#[test]
fn any_plan_placement_gives_the_same_order_index() {
    let roots = [
        corders_group::Root {
            pointer: GROUP,
            handle: HANDLE,
            country: 0,
        },
        corders_group::Root {
            pointer: GROUP + 0x100,
            handle: HANDLE + 1,
            country: 0,
        },
    ];
    let visits = [
        order::Visit::fixture(PARA + 0x300, 1, 4),
        order::Visit::default(),
        order::Visit::fixture(PARA, 0, 9),
        order::Visit::fixture(PARA + 0x100, 0, 2),
        order::Visit::default(),
    ];
    let rows =
        [(HANDLE, 2), (HANDLE, 9), (HANDLE + 1, 4)].map(|(owner_group, instance)| order::Row {
            owner_group,
            instance,
            ..Default::default()
        });
    let index = |visits: &[order::Visit], plan_rows: &[u32]| {
        let mut packet = Prepared::try_new(capacity()).unwrap();
        packet.index_orders(Orders {
            plan_rows,
            ..orders(&roots, visits, &rows)
        });
        (packet.visits.clone(), packet.index_error)
    };
    let census = index(&visits, &[]);
    assert_eq!(census.1, None);
    assert_eq!(
        census.0.iter().map(|entry| entry.1).collect::<Vec<_>>(),
        [3, 2, 0]
    );
    for plan_rows in [&[2, 1, 0][..], &[0, 1, 2], &[2, 2, 0], &[2], &[2, 1, 7]] {
        assert_eq!(index(&visits, plan_rows), census, "{plan_rows:?}");
    }
    let mut orphaned = visits;
    orphaned[3] = order::Visit::fixture(PARA + 0x100, 5, 2);
    let failed = index(&orphaned, &[]);
    assert_eq!(failed.1, Some("execution order owner missing"));
    assert_eq!(index(&orphaned, &[2, 1, 0]).1, failed.1);
    let short = index(&visits[..3], &[]);
    assert_eq!(short.1, Some("execution visit census count mismatch"));
    assert_eq!(index(&visits[..3], &[1, 0, 2]).1, short.1);
}

#[test]
fn raw_nonboolean_and_extreme_progress_survive_later_preparation_refusal() {
    let mut image = Image::default();
    image.put(PARA + order::CONDITION_CACHE, &[0xff]);
    image.word(PARA + order::ACCUMULATED_PREPARATION, i64::MIN as u64);
    image.reserved = Some(Err("reserved source failed"));
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [order::Visit::fixture(PARA, 0, 2)];
    let rows = [order::Row {
        owner_group: HANDLE,
        instance: 2,
        order_type: order::TYPE_PARADROP,
        ..Default::default()
    }];
    let mut plans = [PlanRow {
        owner_group: HANDLE,
        instance: 2,
        ..Default::default()
    }];
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(1, 0).unwrap();
    memo.reset(BASE, STATE);
    let countries = memo.view(&image, BASE, STATE);
    let mut packet = Prepared::try_new(capacity()).unwrap();
    packet.capture_orders(
        &image,
        BASE,
        STATE,
        orders(&roots, &visits, &rows),
        &mut plans,
        Outcome::Rows { len: 1 },
        &countries,
    );
    assert_eq!(
        plans[0].execution,
        Some(OrderExecutionInputs {
            route_is_ok: Some(0xff),
            paradrop: Some(ParadropPreparationInputs {
                progress: i64::MIN,
                terms: None
            })
        })
    );
    assert_eq!(
        &*image.reads.borrow(),
        &[
            (PARA + order::CONDITION_CACHE, 1),
            (PARA + order::ACCUMULATED_PREPARATION, 8)
        ]
    );
}

#[test]
fn scalar_growth_retains_original_successes_and_faults() {
    let mut image = Image::default();
    for index in 0..50usize {
        if !index.is_multiple_of(10) {
            image.word(0x10_0000 + index * 16, index as u64);
        }
    }
    let prepared = Prepared::try_new(Capacity::default()).unwrap();
    let mut bytes = [0u8; 8];
    for _ in 0..2 {
        for index in 0..50usize {
            assert_eq!(
                prepared
                    .scalars
                    .copy(&image, 0x10_0000 + index * 16, &mut bytes),
                !index.is_multiple_of(10)
            );
            if !index.is_multiple_of(10) {
                assert_eq!(u64::from_le_bytes(bytes), index as u64);
            }
        }
    }
    assert_eq!(image.reads.borrow().len(), 50);
    assert_eq!(prepared.work().scalar_reads, 50);
    assert_eq!(prepared.work().scalar_failed, 5);
    assert_eq!(prepared.required_capacity().scalar_observations, 50);
    assert!(prepared.capacity().scalar_observations >= 50);
}

#[test]
fn cold_paradrop_terms_grow_from_original_scalars_and_modifier_headers() {
    let mut image = Image::default();
    image.put(PARA + order::CONDITION_CACHE, &[1]);
    image.word(PARA + order::ACCUMULATED_PREPARATION, 10);
    image.put(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, &[0; 16]);
    image.word(GROUP + corders_group::LEADER, 0);
    image.reserved = Some(Ok(engine_native_memory::country_identity::Country {
        pointer: COUNTRY,
        canonical_slot: 0,
        id: 0,
    }));
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [order::Visit::fixture(PARA, 0, 2)];
    let rows = [order::Row {
        owner_group: HANDLE,
        instance: 2,
        order_type: order::TYPE_PARADROP,
        ..Default::default()
    }];
    let mut plans = [PlanRow {
        owner_group: HANDLE,
        instance: 2,
        ..Default::default()
    }];
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(1, 0).unwrap();
    memo.reset(BASE, STATE);
    let countries = memo.view(&image, BASE, STATE);
    let mut packet = Prepared::vacant();
    packet.capture_orders(
        &image,
        BASE,
        STATE,
        orders(&roots, &visits, &rows),
        &mut plans,
        Outcome::Rows { len: 1 },
        &countries,
    );
    assert_eq!(
        plans[0].execution.unwrap().paradrop.unwrap().terms,
        Some(ParadropPreparationTerms {
            target: 0,
            country_term: 0,
            relation_term: None,
            leader_term: None,
            days_term: 0,
        })
    );
    assert_eq!(packet.required_capacity().scalar_observations, 1);
    assert_eq!(packet.required_capacity().modifiers.nodes, 1);
    assert_eq!(
        image
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == GROUP + corders_group::LEADER)
            .count(),
        1
    );
    assert_eq!(
        image
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
            .count(),
        1
    );
}

#[test]
fn failed_scalar_is_observed_once_and_short_storage_grows_to_observe_the_next_once() {
    let mut image = Image::default();
    image.put(20, &[5]);
    let scalars = Scalars::new(1).unwrap();
    let mut output = [0; 1];
    assert!(!scalars.copy(&image, 10, &mut output));
    assert!(!scalars.copy(&image, 10, &mut output));
    assert!(scalars.copy(&image, 20, &mut output));
    assert_eq!(output, [5]);
    output = [0];
    assert!(scalars.copy(&image, 20, &mut output));
    assert_eq!(output, [5]);
    assert_eq!(&*image.reads.borrow(), &[(10, 1), (20, 1)]);
    let state = scalars.0.borrow();
    assert_eq!((state.required, state.failed), (2, 1));
}

#[test]
fn stat_identity_is_retained_for_both_leaves_and_aliased_sources() {
    for vtable in [
        None,
        Some(0),
        Some((BASE + csub_unit_definition::VTABLE_RVA.0) as u64),
    ] {
        let mut image = Image::default();
        if let Some(vtable) = vtable {
            image.word(STAT, vtable);
        }
        image.word(STAT + NAVAL_WEIGHT, 123);
        image.put(STAT + PARADROP_ELIGIBLE, &[7]);
        let seeds = [
            seed(),
            carmy::DivisionSeed {
                pointer: ARMY + 0x20000,
                ..seed()
            },
        ];
        let mut prepared = Prepared::try_new(capacity()).unwrap();
        prepared.base = BASE;
        let hosts = AssignedSources::fixture(Vec::new());
        let memory = UnitMemory {
            memory: &image,
            prepared: &prepared,
            seeds: &seeds,
            orders: orders(&[], &[], &[]),
            plans: source(&[], &[], &[], &[], &hosts),
        };
        let reason = match vtable {
            None => Some(STAT_REFUSALS[1]),
            Some(0) => Some(STAT_REFUSALS[2]),
            _ => None,
        };
        for seed in &seeds {
            assert_eq!(
                memory.stat::<i64>(seed, NAVAL_WEIGHT),
                reason.map_or(Ok(123), Err)
            );
            assert_eq!(
                memory.stat::<u8>(seed, PARADROP_ELIGIBLE),
                reason.map_or(Ok(7), Err)
            );
        }
        let reads = image.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, _)| address == STAT)
                .count(),
            1
        );
        if let Some(reason) = reason {
            assert_eq!(&*reads, &[(STAT, 8)]);
            let index = STAT_REFUSALS
                .iter()
                .position(|&name| name == reason)
                .unwrap();
            assert_eq!(prepared.work().stat_refusals[index], 4);
        } else {
            assert_eq!(
                &*reads,
                &[
                    (STAT, 8),
                    (STAT + NAVAL_WEIGHT, 8),
                    (STAT + PARADROP_ELIGIBLE, 1)
                ]
            );
        }
    }
}

#[test]
fn naval_stat_fault_preserves_para_sibling_and_seed_fields_are_not_reread() {
    let mut image = Image::default();
    image.word(STAT, (BASE + csub_unit_definition::VTABLE_RVA.0) as u64);
    image.put(STAT + PARADROP_ELIGIBLE, &[7]);
    image.put(ARMY + member::HOST_OFFSET + member::ORDER_CACHE_DIRTY, &[1]);
    image.word(ARMY + member::HOST_OFFSET + member::ROOT_ORDER, 0);
    image.word(GROUP, (BASE + corders_group::VTABLE_RVA.0) as u64);
    image.word(GROUP + corders_group::HANDLE, HANDLE);
    image.word(GROUP + corders_group::LEADER, 0);
    image.word(PROVINCE, (BASE + cprovince::VTABLE_RVA.0) as u64);
    image.word(PROVINCE + cprovince::DEFINITION, DEFINITION as u64);
    image.put(PROVINCE + cprovince::ID, &7_u32.to_le_bytes());
    image.put(
        DEFINITION + engine_native::cprovince::definition::ID,
        &7_u32.to_le_bytes(),
    );
    image.put(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, &[0; 16]);
    image.reserved = Some(Ok(engine_native_memory::country_identity::Country {
        canonical_slot: 0,
        id: 0,
        pointer: COUNTRY,
    }));
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [
        order::Visit::fixture(NAVAL, 0, 1),
        order::Visit::fixture(PARA, 0, 2),
    ];
    let rows = [
        order::Row {
            owner_group: HANDLE,
            instance: 1,
            order_type: order::TYPE_INVASION,
            ..Default::default()
        },
        order::Row {
            owner_group: HANDLE,
            instance: 2,
            order_type: order::TYPE_PARADROP,
            paradrop_target: 7,
            ..Default::default()
        },
    ];
    let plans = [
        PlanRow {
            owner_group: HANDLE,
            instance: 1,
            assigned_len: 1,
            ..Default::default()
        },
        PlanRow {
            owner_group: HANDLE,
            instance: 2,
            assigned_start: 1,
            assigned_len: 1,
            execution: Some(OrderExecutionInputs {
                route_is_ok: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        },
    ];
    let groups = [corders_group::PlanRow {
        group: HANDLE,
        primary_len: 1,
        ..Default::default()
    }];
    let hosts = AssignedSources::fixture(vec![ARMY, ARMY]);
    let seeds = [seed()];
    let o = orders(&roots, &visits, &rows);
    let mut packet = Prepared::try_new(capacity()).unwrap();
    packet.base = BASE;
    packet.state = STATE;
    packet.bound = true;
    packet.index_orders(o);
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(1, 0).unwrap();
    memo.reset(BASE, STATE);
    packet.capture_units(
        &image,
        BASE,
        STATE,
        &seeds,
        Outcome::Rows { len: 1 },
        o,
        source(&plans, &groups, &[2], &[UNIT, UNIT], &hosts),
        &memo.view(&image, BASE, STATE),
    );
    assert_eq!(
        packet.units.rows(),
        &[UnitRow {
            division: UNIT,
            inputs: Some(DivisionOrderTransportInputs {
                naval_weight: None,
                paradrop_eligible: Some(7),
                paradrop_modifiers: Some(ParadropTransportModifiers {
                    country_term: 0,
                    commander_term: None
                })
            })
        }]
    );
    assert_eq!(packet.work().naval_demand, 1);
    assert_eq!(packet.work().naval_available, 0);
    for offset in [
        0,
        carmy::SECONDARY_BASE,
        cunit::HANDLE,
        cunit::CLASS,
        carmy::ORDERS_GROUP,
        cunit::ROSTER_OWNER,
        cunit::LOCATION,
    ] {
        assert!(
            !image
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == ARMY + offset)
        );
    }
}

#[test]
fn full_handle_pointer_alias_is_explicit_unavailable_without_native_stat_reads() {
    let image = Image::default();
    let first = seed();
    let mut alias = first;
    alias.pointer += 0x20000;
    let seeds = [first, alias];
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [order::Visit::fixture(NAVAL, 0, 1)];
    let rows = [order::Row {
        owner_group: HANDLE,
        instance: 1,
        order_type: order::TYPE_INVASION,
        ..Default::default()
    }];
    let plans = [PlanRow {
        owner_group: HANDLE,
        instance: 1,
        assigned_len: 1,
        ..Default::default()
    }];
    let hosts = AssignedSources::fixture(vec![ARMY]);
    let o = orders(&roots, &visits, &rows);
    let mut packet = Prepared::try_new(capacity()).unwrap();
    packet.base = BASE;
    packet.state = STATE;
    packet.bound = true;
    packet.index_orders(o);
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
    memo.reset(BASE, STATE);
    packet.capture_units(
        &image,
        BASE,
        STATE,
        &seeds,
        Outcome::Rows { len: 2 },
        o,
        source(&plans, &[], &[], &[UNIT], &hosts),
        &memo.view(&image, BASE, STATE),
    );
    assert_eq!(
        packet.units.rows(),
        &[UnitRow {
            division: UNIT,
            inputs: None
        }]
    );
    assert!(image.reads.borrow().is_empty());
}

#[test]
fn root_hook_preserves_stale_failure_and_never_uses_native_list_fallback() {
    let mut image = Image::default();
    image.put(ARMY + member::HOST_OFFSET + member::ORDER_CACHE_DIRTY, &[0]);
    image.word(ARMY + member::HOST_OFFSET + member::ROOT_ORDER, PARA as u64);
    let mut seed = seed();
    seed.orders_group = Some(0);
    let seeds = [seed];
    let hosts = AssignedSources::fixture(Vec::new());
    let o = orders(&[], &[], &[]);
    let mut prepared = Prepared::try_new(capacity()).unwrap();
    prepared.base = BASE;
    prepared.state = STATE;
    prepared.bound = true;
    prepared.index_orders(o);
    prepared.index_units(&seeds);
    let p = source(&[], &[], &[], &[], &hosts);
    prepared.index_memberships(&seeds, o, p);
    let memory = UnitMemory {
        memory: &image,
        prepared: &prepared,
        seeds: &seeds,
        orders: o,
        plans: p,
    };
    let member = ARMY + member::HOST_OFFSET;
    assert_eq!(
        memory.movement_primary_root(BASE, member),
        Some(Err("movement primary root clean cache is stale"))
    );
    assert_eq!(
        memory.movement_primary_root(BASE, member),
        Some(Err("movement primary root clean cache is stale"))
    );
    assert!(
        memory
            .movement_primary_root(BASE, usize::MAX)
            .unwrap()
            .is_err()
    );
    assert_eq!(
        &*image.reads.borrow(),
        &[
            (member + member::ORDER_CACHE_DIRTY, 1),
            (member + member::ROOT_ORDER, 8)
        ]
    );
}

#[test]
fn first_own_primary_precedes_only_the_immediate_parent() {
    const PARENT: usize = 0x30000;
    const GRANDPARENT: usize = 0x40000;
    const THIRD: usize = 0x50000;
    let parent_handle = HANDLE + (1_u64 << 32);
    let grandparent_handle = HANDLE + (2_u64 << 32);
    for (own_member, parent_member, expected) in
        [(true, true, NAVAL), (false, true, PARA), (false, false, 0)]
    {
        let mut image = Image::default();
        image.put(ARMY + member::HOST_OFFSET + member::ORDER_CACHE_DIRTY, &[1]);
        image.word(
            ARMY + member::HOST_OFFSET + member::ROOT_ORDER,
            THIRD as u64,
        );
        image.word(GROUP + corders_group::PARENT, PARENT as u64);
        image.word(PARENT + corders_group::PARENT, GRANDPARENT as u64);
        let seeds = [seed()];
        let roots = [
            corders_group::Root {
                pointer: GROUP,
                handle: HANDLE,
                country: 0,
            },
            corders_group::Root {
                pointer: PARENT,
                handle: parent_handle,
                country: 0,
            },
            corders_group::Root {
                pointer: GRANDPARENT,
                handle: grandparent_handle,
                country: 0,
            },
        ];
        let visits = [
            order::Visit::fixture(NAVAL, 0, 1),
            order::Visit::fixture(PARA, 1, 1),
            order::Visit::fixture(THIRD, 2, 1),
        ];
        let rows = [
            order::Row {
                owner_group: HANDLE,
                instance: 1,
                ..Default::default()
            },
            order::Row {
                owner_group: parent_handle,
                instance: 1,
                ..Default::default()
            },
            order::Row {
                owner_group: grandparent_handle,
                instance: 1,
                ..Default::default()
            },
        ];
        let own = usize::from(own_member);
        let parent = usize::from(parent_member);
        let plans = [
            PlanRow {
                owner_group: HANDLE,
                instance: 1,
                assigned_len: own,
                ..Default::default()
            },
            PlanRow {
                owner_group: parent_handle,
                instance: 1,
                assigned_start: own,
                assigned_len: parent,
                ..Default::default()
            },
            PlanRow {
                owner_group: grandparent_handle,
                instance: 1,
                assigned_start: own + parent,
                assigned_len: 1,
                ..Default::default()
            },
        ];
        let groups = [
            corders_group::PlanRow {
                group: HANDLE,
                primary_len: 1,
                ..Default::default()
            },
            corders_group::PlanRow {
                group: parent_handle,
                primary_start: 1,
                primary_len: 1,
                ..Default::default()
            },
            corders_group::PlanRow {
                group: grandparent_handle,
                primary_start: 2,
                primary_len: 1,
                ..Default::default()
            },
        ];
        let assigned = vec![UNIT; own + parent + 1];
        let hosts = AssignedSources::fixture(vec![ARMY; assigned.len()]);
        let o = orders(&roots, &visits, &rows);
        let p = source(&plans, &groups, &[1, 1, 1], &assigned, &hosts);
        let mut prepared = Prepared::try_new(capacity()).unwrap();
        prepared.base = BASE;
        prepared.state = STATE;
        prepared.bound = true;
        prepared.index_orders(o);
        prepared.index_units(&seeds);
        prepared.index_memberships(&seeds, o, p);
        let memory = UnitMemory {
            memory: &image,
            prepared: &prepared,
            seeds: &seeds,
            orders: o,
            plans: p,
        };
        assert_eq!(
            memory.movement_primary_root(BASE, ARMY + member::HOST_OFFSET),
            Some(Ok(expected))
        );
        assert!(
            !image
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == PARENT + corders_group::PARENT)
        );
        assert_eq!(
            image
                .reads
                .borrow()
                .iter()
                .filter(|&&(address, _)| address == GROUP + corders_group::PARENT)
                .count(),
            usize::from(!own_member)
        );
    }
}

#[test]
fn mid_read_panic_restores_buffers_and_same_capacity_next_pass_recovers() {
    struct PanicOnce<'a> {
        image: &'a Image,
        armed: Cell<bool>,
    }
    impl Memory for PanicOnce<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == STAT + NAVAL_WEIGHT && self.armed.replace(false) {
                panic!("mid-unit read witness");
            }
            self.image.copy(address, destination)
        }
    }
    let mut image = Image::default();
    image.word(STAT, (BASE + csub_unit_definition::VTABLE_RVA.0) as u64);
    image.word(STAT + NAVAL_WEIGHT, i64::MAX as u64);
    let memory = PanicOnce {
        image: &image,
        armed: Cell::new(true),
    };
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 0,
    }];
    let visits = [order::Visit::fixture(NAVAL, 0, 1)];
    let rows = [order::Row {
        owner_group: HANDLE,
        instance: 1,
        order_type: order::TYPE_INVASION,
        ..Default::default()
    }];
    let plans = [PlanRow {
        owner_group: HANDLE,
        instance: 1,
        assigned_len: 1,
        ..Default::default()
    }];
    let hosts = AssignedSources::fixture(vec![ARMY]);
    let seeds = [seed()];
    let o = orders(&roots, &visits, &rows);
    let p = source(&plans, &[], &[], &[UNIT], &hosts);
    let c = capacity();
    let mut prepared = Prepared::try_new(c).unwrap();
    prepared.base = BASE;
    prepared.state = STATE;
    prepared.bound = true;
    prepared.index_orders(o);
    let before = prepared.allocated_bytes();
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
    memo.reset(BASE, STATE);
    let countries = memo.view(&memory, BASE, STATE);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepared.capture_units(
            &memory,
            BASE,
            STATE,
            &seeds,
            Outcome::Rows { len: 1 },
            o,
            p,
            &countries,
        )
    }));
    assert!(panic.is_err());
    assert_eq!(prepared.allocated_bytes(), before);
    assert_eq!(prepared.attempted.len(), seeds.len());
    prepared.fail_units("parent worker panic");
    assert!(matches!(
        prepared.units.outcome(),
        Outcome::Unavailable { .. }
    ));
    prepared.try_grow(c).unwrap();
    prepared.base = BASE;
    prepared.state = STATE;
    prepared.bound = true;
    prepared.index_orders(o);
    prepared.capture_units(
        &memory,
        BASE,
        STATE,
        &seeds,
        Outcome::Rows { len: 1 },
        o,
        p,
        &countries,
    );
    assert_eq!(
        prepared.units.rows(),
        &[UnitRow {
            division: UNIT,
            inputs: Some(DivisionOrderTransportInputs {
                naval_weight: Some(i64::MAX),
                ..Default::default()
            })
        }]
    );
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
#[ignore = "run alone with alloc-fault-injection: allocation hooks are process-global"]
fn execution_advisory_denial_keeps_the_terminal_hook_installed_and_inactive() {
    use engine_allocation as allocation;
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FAILURES: AtomicUsize = AtomicUsize::new(0);
    fn before() -> bool {
        true
    }
    fn failed() {
        FAILURES.fetch_add(1, Ordering::SeqCst);
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            allocation::set_hooks(None, None);
        }
    }
    let _reset = Reset;
    let mut image = Image::default();
    image.word(BASE + registry::LOW_ROOTS_RVA.0 + 53 * 8, 0x10000);
    let mut header = [0; 17];
    header[..8].copy_from_slice(&0x11000_u64.to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..16].copy_from_slice(&7_u32.to_le_bytes());
    header[16] = 5;
    image.put(0x10000 + registry::ENTRIES, &header);
    let mut buckets = vec![0; 14 * bucket::STRIDE];
    buckets[bucket::DISTANCE] = 1;
    buckets[bucket::TYPE..bucket::OBJECT].copy_from_slice(&HANDLE.to_le_bytes());
    buckets[13 * bucket::STRIDE + bucket::DISTANCE] = 255;
    image.put(0x11000, &buckets);
    image.reads.borrow_mut().reserve(1024);
    allocation::set_hooks(Some(before), Some(failed));
    let hint =
        allocation::test_support::with_allocations_denied(|| demand::demand(&image, BASE, STATE));
    assert_eq!(hint.capacity.groups, 1);
    assert_eq!(FAILURES.load(Ordering::SeqCst), 0);
    assert!(!allocation::capture_stopped());
    assert!(allocation::vec::<u8>(1).is_err());
    assert_eq!(FAILURES.load(Ordering::SeqCst), 1);
}

#[test]
fn execution_modifier_demand_excludes_allocator_spare_capacity() {
    let mut image = Image::default();
    let at = 0x40000;
    let data = 0x41000;
    image.word(at, data);
    image.put(at + 8, &5_001_u32.to_le_bytes());
    image.put(at + 12, &5_001_u32.to_le_bytes());
    let mut entries = vec![0; 5_001 * 16];
    for (index, entry) in entries.chunks_exact_mut(16).enumerate() {
        entry[..4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
    }
    image.put(data as usize, &entries);
    let mut prepared = Prepared::try_new(Capacity::default()).unwrap();
    prepared
        .snapshots
        .get_mut()
        .capture_at(&image, at, 1)
        .unwrap();
    assert_eq!(prepared.capacity().modifiers.entries, 5_001);
    for _ in 0..20 {
        let demand = prepared.capacity();
        prepared.try_grow(demand).unwrap();
        assert_eq!(prepared.capacity(), demand);
    }
}
