use super::{Captured, DivisionsCopyContext, DivisionsCopyReport, DivisionsJob};
use crate::extended::production::complete_source;
use engine_deployment::{
    army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status, cupgrade_delivery,
};
use engine_native::{
    Memory, carmy, ccountry, cequipment_variant, cequipment_variant_pool, cgame_state,
    corders_group, cprovince, creferenced_division_template, cunit, memory::SliceMemory,
    walk::Outcome,
};

pub(crate) const BASE: usize = 0;
pub(crate) const GAME_STATE: usize = 0x10000;
pub(crate) const CTAB: usize = 0x11000;
pub(crate) const LTAB: usize = 0x12000;
pub(crate) const PTAB: usize = 0x13000;
pub(crate) const C0: usize = 0x20000;
pub(crate) const C1: usize = 0x21000;
pub(crate) const P1: usize = 0x24000;
pub(crate) const A1: usize = 0x30000;
pub(crate) const A2: usize = 0x31000;
pub(crate) const A3: usize = 0x32000;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}

fn variant(bytes: &mut [u8], at: usize, id: u32) {
    p64(bytes, at, cequipment_variant::VTABLE_RVA.0 as u64);
    p64(
        bytes,
        at + 8,
        ((id as u64) << 32) | cequipment_variant::REFERENCE_TYPE as u64,
    );
}

pub(crate) fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x68000];
    // Slots 1..2 hold countries; slot 3 aliases the first object.
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
    for (index, pointer) in [0u64, C0 as u64, C1 as u64, C0 as u64]
        .into_iter()
        .enumerate()
    {
        p64(&mut bytes, CTAB + index * 8, pointer);
    }
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
    for (index, slot) in [0u32, 1, 2, 1].into_iter().enumerate() {
        p32(&mut bytes, LTAB + index * 4, slot);
    }
    header(&mut bytes, GAME_STATE + cgame_state::PROVINCES, PTAB, 2);
    p64(&mut bytes, PTAB, 0);
    p64(&mut bytes, PTAB + 8, P1 as u64);
    for (country, id, roster, entries) in [
        (C0, 1u32, 0x23000usize, vec![A1, A2]),
        (C1, 2u32, 0x23100usize, vec![A3]),
    ] {
        p64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, country + ccountry::ID, id);
        header(
            &mut bytes,
            country + ccountry::DIVISIONS,
            roster,
            entries.len() as u32,
        );
        for (index, division) in entries.into_iter().enumerate() {
            p64(&mut bytes, roster + index * 8, division as u64);
        }
    }
    p64(&mut bytes, P1, cprovince::VTABLE_RVA.0 as u64);
    p32(&mut bytes, P1 + cprovince::ID, 1);
    // A1 is complete; A2 has null location and orders; A3 has a null
    // template.
    for (army, handle, owner, location, template, orders, hp, org) in [
        (
            A1,
            101u64,
            1u32,
            P1,
            0x40000usize,
            0x41000usize,
            100_000i64,
            50_000i64,
        ),
        (
            A2,
            102u64,
            1u32,
            0,
            0x40000usize,
            0usize,
            200_000i64,
            60_000i64,
        ),
        (
            A3,
            103u64,
            2u32,
            P1,
            0usize,
            0x41000usize,
            -300_000i64,
            70_000i64,
        ),
    ] {
        p64(&mut bytes, army, carmy::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            army + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        p64(&mut bytes, army + cunit::HANDLE, handle);
        p32(&mut bytes, army + cunit::ROSTER_OWNER, owner);
        p32(&mut bytes, army + cunit::LOGICAL_OWNER, owner);
        p32(&mut bytes, army + cunit::EXPEDITIONARY_OWNER, 0);
        p64(&mut bytes, army + cunit::LOCATION, location as u64);
        p64(&mut bytes, army + carmy::STANDARD_TEMPLATE, template as u64);
        p64(&mut bytes, army + carmy::STRENGTH, hp as u64);
        p64(&mut bytes, army + carmy::ORGANISATION, org as u64);
        p64(&mut bytes, army + carmy::ORDERS_GROUP, orders as u64);
    }
    p64(
        &mut bytes,
        0x40000,
        creferenced_division_template::VTABLE_RVA.0 as u64,
    );
    p64(
        &mut bytes,
        0x40000 + creferenced_division_template::TYPE,
        0x3455_0000_0034u64,
    );
    p64(&mut bytes, 0x41000, corders_group::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        0x41008,
        ((57u64) << 32) | corders_group::REFERENCE_TYPE as u64,
    );
    // Equipment pools: two signed entries, one entry, and empty.
    variant(&mut bytes, 0x50000, 11);
    variant(&mut bytes, 0x51000, 12);
    for (army, items, entries) in [
        (
            A1,
            0x42000usize,
            vec![(0x50000usize, 100i64), (0x51000usize, -50i64)],
        ),
        (A2, 0x42100usize, vec![(0x50000usize, 7i64)]),
        (A3, 0x42200usize, vec![]),
    ] {
        let pool = army + carmy::EQUIPMENT_POOL;
        p64(
            &mut bytes,
            pool,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        header(
            &mut bytes,
            pool + cequipment_variant_pool::ITEMS,
            items,
            entries.len() as u32,
        );
        for (index, (pointer, amount)) in entries.into_iter().enumerate() {
            p64(&mut bytes, items + index * 16, pointer as u64);
            p64(&mut bytes, items + index * 16 + 8, amount as u64);
        }
    }
    // Upgrade groups: live group with one request and one delivery,
    // empty group, and a group with a request but no deliveries.
    for (army, group, priority, requests, deliveries) in [
        (
            A1,
            0x60070usize,
            3u32,
            vec![(0x50000usize, 1000i64, 2000i64)],
            vec![0x64000usize],
        ),
        (A2, 0x61070usize, 5u32, vec![], vec![]),
        (
            A3,
            0x62070usize,
            7u32,
            vec![(0x51000usize, 50i64, 60i64)],
            vec![],
        ),
    ] {
        p64(&mut bytes, army + carmy::REQUESTS, (group - 0x70) as u64);
        p64(
            &mut bytes,
            group,
            carmy_upgrades_requests::VTABLE_RVA.0 as u64,
        );
        p32(
            &mut bytes,
            group + carmy_upgrades_requests::PRIORITY,
            priority,
        );
        p64(
            &mut bytes,
            group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
            carmy_upgrades_requests::PERSISTENT_VTABLE_RVA.0 as u64,
        );
        p64(
            &mut bytes,
            group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT + 8,
            army as u64,
        );
        header(
            &mut bytes,
            group + carmy_upgrades_requests::REQUESTS_DATA,
            group + 0x200,
            requests.len() as u32,
        );
        for (index, (pointer, request, total)) in requests.into_iter().enumerate() {
            let at = group + 0x200 + index * 0x18;
            p64(&mut bytes, at, pointer as u64);
            p64(&mut bytes, at + 8, request as u64);
            p64(&mut bytes, at + 16, total as u64);
        }
        header(
            &mut bytes,
            group + carmy_upgrades_requests::DELIVERIES_DATA,
            group + 0x300,
            deliveries.len() as u32,
        );
        for (index, delivery) in deliveries.into_iter().enumerate() {
            p64(&mut bytes, group + 0x300 + index * 8, delivery as u64);
        }
    }
    // The one live delivery: progress-available status with a produced row.
    p64(&mut bytes, 0x64000, cupgrade_delivery::VTABLE_RVA.0 as u64);
    p32(&mut bytes, 0x64000 + cupgrade_delivery::STATUS, 1);
    p64(&mut bytes, 0x64000 + cupgrade_delivery::PROGRESS, 300);
    p64(&mut bytes, 0x64000 + cupgrade_delivery::TOTAL_PROGRESS, 400);
    p64(
        &mut bytes,
        0x64020,
        cequipment_variant_pool::VTABLE_RVA.0 as u64,
    );
    header(
        &mut bytes,
        0x64020 + cequipment_variant_pool::ITEMS,
        0x64100,
        1,
    );
    p64(&mut bytes, 0x64100, 0x51000);
    p64(&mut bytes, 0x64108, 9);
    p64(&mut bytes, 0x64020 + cequipment_variant_pool::ALLOW_ZERO, 1);
    bytes
}

pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub(crate) struct Split {
    pub divisions_outcome: Outcome,
    pub divisions: Vec<carmy::Row>,
    pub equipment_outcome: Outcome,
    pub equipment: Vec<cequipment_variant_pool::DivisionRow>,
    pub groups_outcome: Outcome,
    pub groups: Vec<carmy_upgrades_requests::Row>,
    pub requests_outcome: Outcome,
    pub requests: Vec<army_upgrade_request_row::Row>,
    pub deliveries_outcome: Outcome,
    pub deliveries: Vec<cupgrade_delivery::Row>,
    pub produced_outcome: Outcome,
    pub produced: Vec<cupgrade_delivery::ProducedRow>,
}

/// Reference typed walks with identical pooled buffers, mirroring the
/// production acquire's equipment/upgrade sections exactly.
pub(crate) fn direct(memory: &impl Memory, lens: [usize; 6]) -> Split {
    let mut divisions = vec![carmy::Row::default(); lens[0]];
    let mut visited = Vec::new();
    let divisions_outcome =
        carmy::walk_with(memory, BASE, GAME_STATE, &mut divisions, |pointer, row| {
            visited.push(cequipment_variant_pool::DivisionSource {
                pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            });
        });
    let first_sources = complete_source(&visited, divisions_outcome);
    let mut equipment = vec![cequipment_variant_pool::DivisionRow::default(); lens[1]];
    let equipment_outcome = match first_sources {
        Ok(source) => cequipment_variant_pool::walk_divisions(memory, BASE, source, &mut equipment),
        Err(reason) => Outcome::Unavailable { reason },
    };
    let mut groups = vec![carmy_upgrades_requests::Row::default(); lens[2]];
    let mut requests = vec![army_upgrade_request_row::Row::default(); lens[3]];
    let mut deliveries = vec![cupgrade_delivery::Row::default(); lens[4]];
    let mut produced = vec![cupgrade_delivery::ProducedRow::default(); lens[5]];
    let result = match first_sources {
        Ok(source) => carmy_upgrades_status::walk(
            memory,
            BASE,
            source,
            &mut groups,
            &mut requests,
            &mut deliveries,
            &mut produced,
        ),
        Err(reason) => carmy_upgrades_status::Outcomes::unavailable(reason),
    };
    Split {
        divisions_outcome,
        divisions,
        equipment_outcome,
        equipment,
        groups_outcome: result.groups,
        groups,
        requests_outcome: result.requests,
        requests,
        deliveries_outcome: result.deliveries,
        deliveries,
        produced_outcome: result.produced,
        produced,
    }
}

/// One owned acquisition installed into a pooled frame with equal capacities.
pub(crate) fn split(
    memory: &impl Memory,
    lens: [usize; 6],
) -> (Split, DivisionsCopyReport, DivisionsJob) {
    let ctx = DivisionsCopyContext {
        base: BASE,
        game_state: GAME_STATE,
        row_capacity: lens[0],
        source_capacity: lens[0],
        detail_capacities: lens[1..].try_into().unwrap(),
        details_active: true,
    };
    let mut first = DivisionsJob::new();
    let report = super::copy_divisions_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut frame = Captured::new();
    frame.divisions = vec![carmy::Row::default(); lens[0]];
    frame.extended.test_resize_division_buffers(lens);
    let mut ticks = [0u64; 6];
    super::decode_divisions_payload(&mut frame, BASE, &mut first, &mut ticks);
    let detail = frame.extended.test_division_detail();
    // The worker clears the raw-mode gate it consumed.
    assert!(!detail.divisions_raw_mode);
    (
        Split {
            divisions_outcome: frame.divisions_result,
            divisions: std::mem::take(&mut frame.divisions),
            equipment_outcome: detail.division_equipment.outcome,
            equipment: std::mem::take(&mut detail.division_equipment.rows),
            groups_outcome: detail.groups.outcome,
            groups: std::mem::take(&mut detail.groups.rows),
            requests_outcome: detail.requests.outcome,
            requests: std::mem::take(&mut detail.requests.rows),
            deliveries_outcome: detail.deliveries.outcome,
            deliveries: std::mem::take(&mut detail.deliveries.rows),
            produced_outcome: detail.produced.outcome,
            produced: std::mem::take(&mut detail.produced.rows),
        },
        report,
        first,
    )
}

pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
    // Owned transfers retain different storage headroom; outcomes certify
    // the rows to compare, including every retained truncation prefix.
    assert_eq!(
        split.divisions_outcome, direct.divisions_outcome,
        "divisions outcome"
    );
    assert_eq!(
        &split.divisions[..split.divisions_outcome.len()],
        &direct.divisions[..direct.divisions_outcome.len()],
        "divisions rows"
    );
    assert_eq!(
        split.equipment_outcome, direct.equipment_outcome,
        "equipment outcome"
    );
    assert_eq!(
        &split.equipment[..split.equipment_outcome.len()],
        &direct.equipment[..direct.equipment_outcome.len()],
        "equipment rows"
    );
    assert_eq!(
        split.groups_outcome, direct.groups_outcome,
        "groups outcome"
    );
    assert_eq!(
        &split.groups[..split.groups_outcome.len()],
        &direct.groups[..direct.groups_outcome.len()],
        "groups rows"
    );
    assert_eq!(
        split.requests_outcome, direct.requests_outcome,
        "requests outcome"
    );
    assert_eq!(
        &split.requests[..split.requests_outcome.len()],
        &direct.requests[..direct.requests_outcome.len()],
        "requests rows"
    );
    assert_eq!(
        split.deliveries_outcome, direct.deliveries_outcome,
        "deliveries outcome"
    );
    assert_eq!(
        &split.deliveries[..split.deliveries_outcome.len()],
        &direct.deliveries[..direct.deliveries_outcome.len()],
        "deliveries rows"
    );
    assert_eq!(
        split.produced_outcome, direct.produced_outcome,
        "produced outcome"
    );
    assert_eq!(
        &split.produced[..split.produced_outcome.len()],
        &direct.produced[..direct.produced_outcome.len()],
        "produced rows"
    );
}
