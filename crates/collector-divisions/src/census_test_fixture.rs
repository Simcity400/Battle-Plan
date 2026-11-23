use engine_deployment::{carmy_upgrades_requests, cupgrade_delivery};
use engine_native::{
    carmy,
    ccountry,
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cgame_state,
    corders_group,
    cprovince,
    creferenced_division_template,
    cunit,
};
use engine_native_memory::memory::SliceMemory;

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
pub const CTAB: usize = 0x11000;
pub const LTAB: usize = 0x12000;
pub const PTAB: usize = 0x13000;
pub const C0: usize = 0x20000;
pub const C1: usize = 0x21000;
pub const P1: usize = 0x24000;
pub const A1: usize = 0x30000;
pub const A2: usize = 0x31000;
pub const A3: usize = 0x32000;

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
    p64(bytes, at, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    p64(
        bytes,
        at + 8,
        ((id as u64) << 32) | cequipment_variant::REFERENCE_TYPE as u64,
    );
}

pub fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x68000];
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

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}
