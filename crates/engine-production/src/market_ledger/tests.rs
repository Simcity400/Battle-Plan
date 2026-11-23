use super::*;
use engine_native_memory::memory::SliceMemory;

const HOLDER: usize = 0x100;
const ITEMS: usize = 0x500;
const VARIANT: usize = 0x800;
const ARCHETYPE: usize = 0x900;
const TRIGGER: usize = 0xa00;
const HEAD: usize = 0xc00;

fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
    bytes[at..at + N].copy_from_slice(&value);
}
fn u64(bytes: &mut [u8], at: usize, value: u64) {
    put(bytes, at, value.to_le_bytes());
}
fn u32(bytes: &mut [u8], at: usize, value: u32) {
    put(bytes, at, value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    u64(bytes, at, data as u64);
    u32(bytes, at + 8, count);
    u32(bytes, at + 12, count);
}
fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

#[test]
fn market_automation_demand_never_reads_inventory_prices_or_subsidies() {
    let native = engine_native_memory::country_identity::Country {
        pointer: 0x1000,
        id: 5000,
        canonical_slot: 5000,
    };
    let mut bytes = vec![0; 0x3000];
    const STATUS: usize = 0x2500;
    const MANAGER: usize = 0x2800;
    u64(
        &mut bytes,
        native.pointer + ccountry::EQUIPMENT_MARKET_STATUS,
        STATUS as u64,
    );
    u64(&mut bytes, STATUS + status::MANAGER, MANAGER as u64);
    u64(&mut bytes, STATUS + COUNTRY_BACKLINK, native.pointer as u64);
    bytes[STATUS + AUTOMATION] = 1;
    struct Selected<'a> {
        memory: SliceMemory<'a>,
        country: usize,
    }
    impl Memory for Selected<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert!(
                [
                    (self.country + ccountry::EQUIPMENT_MARKET_STATUS, 8),
                    (STATUS + status::MANAGER, 8),
                    (STATUS + COUNTRY_BACKLINK, 8),
                    (STATUS + AUTOMATION, 1),
                ]
                .contains(&(address, output.len())),
                "unselected market fact was read"
            );
            self.memory.copy(address, output)
        }
    }
    let mut output = Country::default();
    output.stockpile.equipment.push(Equipment {
        variant: 70,
        amount_raw: -71,
    });
    country(
        &Selected {
            memory: memory(&bytes),
            country: native.pointer,
        },
        0,
        native,
        MANAGER,
        Demand {
            automation: true,
            ..Demand::default()
        },
        &mut output,
    )
    .unwrap();
    assert_eq!(output.country, 5000);
    assert!(output.auto_send_market_access);
    assert_eq!(output.stockpile.equipment[0].amount_raw, -71);
}

#[test]
fn market_empty_demand_reads_no_entity() {
    struct Unreached;
    impl Memory for Unreached {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("held market facts must not touch native entities");
        }
    }
    let native = engine_native_memory::country_identity::Country {
        pointer: 0,
        id: 5000,
        canonical_slot: 5000,
    };
    country(
        &Unreached,
        0,
        native,
        0,
        Demand::default(),
        &mut Country::default(),
    )
    .unwrap();
}

#[test]
fn market_subsidy_demand_keeps_stored_total_independent_of_grant_sum() {
    let native = engine_native_memory::country_identity::Country {
        pointer: 0x1000,
        id: 17,
        canonical_slot: 17,
    };
    const STATUS: usize = 0x2500;
    const MANAGER: usize = 0x2800;
    let mut bytes = vec![0; 0x3000];
    u64(
        &mut bytes,
        native.pointer + ccountry::EQUIPMENT_MARKET_STATUS,
        STATUS as u64,
    );
    u64(&mut bytes, STATUS + status::MANAGER, MANAGER as u64);
    u64(&mut bytes, STATUS + COUNTRY_BACKLINK, native.pointer as u64);
    u64(
        &mut bytes,
        STATUS + COUNTRY_SUBSIDY_TOTAL,
        (-123456_i64) as u64,
    );
    array(&mut bytes, STATUS + COUNTRY_SUBSIDIES, 0, 0);
    let mut output = Country::default();
    country(
        &memory(&bytes),
        0,
        native,
        MANAGER,
        Demand {
            subsidies: true,
            ..Demand::default()
        },
        &mut output,
    )
    .unwrap();
    assert!(output.subsidies.is_empty());
    assert_eq!(output.subsidy_total_raw, -123456);
}
fn pool_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    u64(
        &mut bytes,
        HOLDER,
        cequipment_variant_pool::VTABLE_RVA.0 as u64,
    );
    array(
        &mut bytes,
        HOLDER + cequipment_variant_pool::ITEMS,
        ITEMS,
        3,
    );
    bytes[HOLDER + cequipment_variant_pool::ALLOW_ZERO] = 1;
    u64(&mut bytes, VARIANT, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    u64(&mut bytes, VARIANT + 8, (5000_u64 << 32) | 70);
    for (index, amount) in [0_i64, -9, i64::from(i32::MAX) + 17]
        .into_iter()
        .enumerate()
    {
        u64(&mut bytes, ITEMS + index * 16, VARIANT as u64);
        u64(&mut bytes, ITEMS + index * 16 + 8, amount as u64);
    }
    bytes
}

#[test]
fn market_pool_retains_zero_negative_wide_amounts_and_order_without_pointer_payloads() {
    let bytes = pool_fixture();
    let mut output = Pool::default();
    pool(&memory(&bytes), 0, HOLDER, &mut output).unwrap();
    assert!(output.allow_zero);
    assert_eq!(
        output
            .equipment
            .iter()
            .map(|r| r.amount_raw)
            .collect::<Vec<_>>(),
        [0, -9, i64::from(i32::MAX) + 17]
    );
    assert!(
        output
            .equipment
            .iter()
            .all(|r| r.variant == (5000_u64 << 32) | 70)
    );
    let allocation = output.equipment.as_ptr();
    pool(&memory(&bytes), 0, HOLDER, &mut output).unwrap();
    assert_eq!(allocation, output.equipment.as_ptr());
}

#[test]
fn market_pool_refuses_a_wrong_variant_type_and_an_unreadable_zero_policy() {
    let mut bytes = pool_fixture();
    u32(&mut bytes, VARIANT + 8, 71);
    assert!(pool(&memory(&bytes), 0, HOLDER, &mut Pool::default()).is_err());
    let bytes = pool_fixture();
    let mut short = bytes[..HOLDER + cequipment_variant_pool::ALLOW_ZERO].to_vec();
    array(&mut short, HOLDER + cequipment_variant_pool::ITEMS, 0, 0);
    assert_eq!(
        pool(&memory(&short), 0, HOLDER, &mut Pool::default()),
        Err("market pool zero-entry flag unreadable")
    );
}

fn subsidy_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    array(&mut bytes, HOLDER, ITEMS, 2);
    u64(&mut bytes, ARCHETYPE, cequipment_type::VTABLE_RVA.0 as u64);
    u32(&mut bytes, ARCHETYPE + cequipment_type::TOKEN, 98765);
    for index in 0..2 {
        let entry = ITEMS + index * SUBSIDY_STRIDE;
        u64(
            &mut bytes,
            entry + SUBSIDY_CIC,
            if index == 0 {
                0
            } else {
                (-123456789_i64) as u64
            },
        );
        u64(&mut bytes, entry + SUBSIDY_ARCHETYPE, ARCHETYPE as u64);
    }
    array(&mut bytes, ITEMS + SUBSIDY_CONDITION, 0x700, 3);
    for (index, tag) in [5000, 7, 5000].into_iter().enumerate() {
        u32(&mut bytes, 0x700 + index * 4, tag);
    }
    let second = ITEMS + SUBSIDY_STRIDE;
    bytes[second + SUBSIDY_KIND] = 1;
    u64(&mut bytes, second + SUBSIDY_CONDITION, TRIGGER as u64);
    u64(&mut bytes, TRIGGER, SUBSIDY_TRIGGER_VTABLE_RVA.0 as u64);
    let name = TRIGGER + SUBSIDY_TRIGGER_NAME;
    bytes[name..name + 11].copy_from_slice(b"mod_trigger");
    u64(&mut bytes, name + msvc_string::LENGTH, 11);
    u64(&mut bytes, name + msvc_string::CAPACITY, 15);
    bytes
}

#[test]
fn market_subsidies_keep_both_condition_alternatives_order_and_signed_cic() {
    let bytes = subsidy_fixture();
    let mut output = Vec::new();
    subsidies(&memory(&bytes), 0, HOLDER, &mut output).unwrap();
    assert_eq!(output[0].targets, [5000, 7, 5000]);
    assert_eq!(output[0].archetype, 98765);
    assert_eq!(output[0].cic_raw, 0);
    assert!(output[0].trigger.is_empty());
    assert_eq!(output[1].trigger, b"mod_trigger");
    assert_eq!(output[1].cic_raw, -123456789);
    assert!(output[1].targets.is_empty());
    let targets = output[0].targets.as_ptr();
    subsidies(&memory(&bytes), 0, HOLDER, &mut output).unwrap();
    assert_eq!(targets, output[0].targets.as_ptr());
}

#[test]
fn market_subsidies_refuse_unknown_condition_and_wrong_trigger_identity() {
    let mut bytes = subsidy_fixture();
    bytes[ITEMS + SUBSIDY_KIND] = 2;
    assert_eq!(
        subsidies(&memory(&bytes), 0, HOLDER, &mut Vec::new()),
        Err("market subsidy condition unsupported")
    );
    let mut bytes = subsidy_fixture();
    u64(&mut bytes, TRIGGER, 0);
    assert_eq!(
        subsidies(&memory(&bytes), 0, HOLDER, &mut Vec::new()),
        Err("market subsidy trigger vtable mismatch")
    );
}

fn price_fixture(count: u32) -> Vec<u8> {
    let mut bytes = vec![0; 0x1000 + count as usize * 0x30];
    u64(&mut bytes, HOLDER, HEAD as u64);
    u64(&mut bytes, HOLDER + 8, count as u64);
    let first = if count == 0 { HEAD } else { 0x1000 };
    u64(&mut bytes, HEAD, first as u64);
    u64(&mut bytes, HEAD + 8, first as u64);
    u64(
        &mut bytes,
        HEAD + 16,
        if count == 0 {
            HEAD as u64
        } else {
            (0x1000 + (count as usize - 1) * 0x30) as u64
        },
    );
    bytes[HEAD + 0x19] = 1;
    for index in 0..count as usize {
        let node = 0x1000 + index * 0x30;
        u64(&mut bytes, node, HEAD as u64);
        u64(
            &mut bytes,
            node + 8,
            if index == 0 {
                HEAD as u64
            } else {
                (node - 0x30) as u64
            },
        );
        u64(
            &mut bytes,
            node + 16,
            if index + 1 == count as usize {
                HEAD as u64
            } else {
                (node + 0x30) as u64
            },
        );
        u64(
            &mut bytes,
            node + PRICE_VARIANT,
            ((index as u64 + 1) << 32) | 70,
        );
        u32(&mut bytes, node + PRICE_LEVEL, (index % 3) as u32);
    }
    bytes
}

#[test]
fn market_prices_keep_all_native_entries_including_normal_levels() {
    let bytes = price_fixture(7);
    let mut output = Vec::new();
    prices(&memory(&bytes), HOLDER, &mut output).unwrap();
    assert_eq!(output.len(), 7);
    for (index, row) in output.iter().enumerate() {
        assert_eq!(row.variant, ((index as u64 + 1) << 32) | 70);
        assert_eq!(row.level, (index % 3) as i32);
    }
}

#[test]
fn market_prices_distinguish_empty_from_wrong_enum_and_damaged_membership() {
    let mut output = Vec::new();
    prices(&memory(&price_fixture(0)), HOLDER, &mut output).unwrap();
    assert!(output.is_empty());
    let mut bytes = price_fixture(2);
    u32(&mut bytes, 0x1000 + PRICE_LEVEL, 3);
    assert_eq!(
        prices(&memory(&bytes), HOLDER, &mut output),
        Err("market price level unsupported")
    );
    let mut bytes = price_fixture(2);
    u64(&mut bytes, 0x1030 + 8, HEAD as u64);
    assert_eq!(
        prices(&memory(&bytes), HOLDER, &mut output),
        Err("market price map traversal invalid")
    );
}

#[test]
fn market_contract_prices_and_requested_amounts_are_independent_pools() {
    const CONTRACT: usize = 0x200;
    const PRICE_ITEMS: usize = 0x500;
    const REQUEST_ITEMS: usize = 0x600;
    const HANDLE: u64 = (5000_u64 << 32) | contract::TYPE as u64;
    let mut bytes = vec![0; 0x1000];
    u64(&mut bytes, CONTRACT, contract::VTABLE_RVA.0 as u64);
    u64(&mut bytes, CONTRACT + contract::ID_TYPE, HANDLE);
    for (pool_address, items, amount) in [
        (CONTRACT + DRAFT, PRICE_ITEMS, 99_999_i64),
        (
            CONTRACT + DRAFT + DRAFT_EQUIPMENT,
            REQUEST_ITEMS,
            1_200_000_i64,
        ),
    ] {
        u64(
            &mut bytes,
            pool_address,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        array(
            &mut bytes,
            pool_address + cequipment_variant_pool::ITEMS,
            items,
            1,
        );
        u64(&mut bytes, items, VARIANT as u64);
        u64(&mut bytes, items + 8, amount as u64);
    }
    u64(&mut bytes, VARIANT, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    u64(&mut bytes, VARIANT + 8, (1234_u64 << 32) | 70);
    array(&mut bytes, CONTRACT + DRAFT + DRAFT_SUBSIDIES, 0, 0);
    let levels = CONTRACT + DRAFT + DRAFT_PRICE_LEVELS;
    u64(&mut bytes, levels, HEAD as u64);
    for offset in [0, 8, 16] {
        u64(&mut bytes, HEAD + offset, HEAD as u64);
    }
    bytes[HEAD + 0x19] = 1;
    let mut terms = Draft::default();
    let mut agreed_prices = Pool::default();
    contract_terms(
        &memory(&bytes),
        0,
        CONTRACT,
        HANDLE,
        &mut terms,
        &mut agreed_prices,
    )
    .unwrap();
    assert_eq!(terms.equipment.equipment[0].amount_raw, 1_200_000);
    assert_eq!(agreed_prices.equipment[0].amount_raw, 99_999);
    assert_eq!(
        terms.equipment.equipment[0].variant,
        agreed_prices.equipment[0].variant
    );
    assert_eq!(
        contract_terms(
            &memory(&bytes),
            0,
            CONTRACT,
            HANDLE + (1_u64 << 32),
            &mut terms,
            &mut agreed_prices
        ),
        Err("market terms contract identity mismatch")
    );
}
