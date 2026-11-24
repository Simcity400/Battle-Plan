use super::*;
use engine_native::cprovince;
use crate::craid_source;
use engine_native_memory::memory::SliceMemory;

const GS: usize = 0x1000;
const ROOT: usize = 0x2000;
const PARENTS: usize = 0x3000;
const CHILDREN: usize = 0x4000;
const INSTANCE: usize = 0x5000;
const COUNTRY: usize = 0x6000;
const COUNTRY_ARRAY: usize = 0x7000;
const LINKS: usize = 0x7100;
const PROVINCE: usize = 0x8000;
const TYPES: usize = 0x8800;
const DEFINITION: usize = 0x8900;
const SHOWN: usize = 0x8a00;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address..address + value.len()].copy_from_slice(value);
}

fn header(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, &(data as u64).to_le_bytes());
    put(bytes, address + 8, &count.to_le_bytes());
    put(bytes, address + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x9000];
    put(
        &mut bytes,
        GS + cgame_state::RAID_SYSTEM,
        &(ROOT as u64).to_le_bytes(),
    );
    put(&mut bytes, ROOT, &(VTABLE_RVA.0 as u64).to_le_bytes());
    header(&mut bytes, ROOT + COUNTRY_STATUSES, PARENTS, 2);
    header(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRY_ARRAY, 4);
    header(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 4);
    put(
        &mut bytes,
        COUNTRY_ARRAY + 8,
        &(COUNTRY as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY_ARRAY + 24,
        &(COUNTRY as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::ID, &3_u32.to_le_bytes());
    put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
    put(&mut bytes, LINKS + 12, &1_u32.to_le_bytes());
    for ordinal in 0..2 {
        let parent = PARENTS + ordinal * ccountry_raid_status::STRIDE;
        put(
            &mut bytes,
            parent,
            &(ccountry_raid_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            parent + ccountry_raid_status::COUNTRY,
            &(if ordinal == 0 { -1_i32 } else { 3_i32 }).to_le_bytes(),
        );
    }
    let parent = PARENTS + ccountry_raid_status::STRIDE;
    header(
        &mut bytes,
        parent + ccountry_raid_status::ELIGIBLE_TYPES,
        TYPES,
        2,
    );
    header(
        &mut bytes,
        parent + ccountry_raid_status::USED_TYPES,
        TYPES,
        1,
    );
    put(&mut bytes, TYPES, &(DEFINITION as u64).to_le_bytes());
    put(&mut bytes, TYPES + 8, &(DEFINITION as u64).to_le_bytes());
    put(
        &mut bytes,
        DEFINITION,
        &(crate::raid_database::TYPE_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, DEFINITION + 8, &0x4242_u32.to_le_bytes());
    header(
        &mut bytes,
        INSTANCE + craid_instance::SHOW_FOR_COUNTRIES,
        SHOWN,
        2,
    );
    put(&mut bytes, SHOWN, &3_u32.to_le_bytes());
    put(&mut bytes, SHOWN + 4, &9_u32.to_le_bytes());
    header(
        &mut bytes,
        PARENTS + ccountry_raid_status::STRIDE + ccountry_raid_status::INSTANCES,
        CHILDREN,
        1,
    );
    put(&mut bytes, CHILDREN, &(INSTANCE as u64).to_le_bytes());
    put(
        &mut bytes,
        INSTANCE,
        &(craid_instance::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        INSTANCE + craid_instance::REFERENCE_TYPE,
        &84_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        INSTANCE + craid_instance::ID,
        &u32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        INSTANCE + craid_instance::PHASE,
        &3_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        INSTANCE + craid_instance::DISTANCE,
        &2_210_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        INSTANCE + craid_instance::PREPARATION_HOURS,
        &360_i32.to_le_bytes(),
    );
    let source = INSTANCE + craid_instance::SOURCE;
    put(
        &mut bytes,
        source + craid_source::KIND,
        &5_u32.to_le_bytes(),
    );
    put(&mut bytes, source + craid_source::KIND_PRESENT, &[1]);
    put(
        &mut bytes,
        source + craid_source::BUILDING_LOCATION,
        &7301_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        source + craid_source::BUILDING_TYPE,
        &77_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        source + craid_source::BUILDING_COUNTRY,
        &(-1_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        source + craid_source::PROVINCE,
        &(PROVINCE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        source + craid_source::SHIP_REFERENCE_TYPE,
        &u32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        source + craid_source::SHIP_REFERENCE_ID,
        &0x8765_4321_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        source + craid_source::COUNTRY,
        &1_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROVINCE,
        &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, PROVINCE + cprovince::ID, &450_u32.to_le_bytes());
    bytes
}

fn read_fixture(
    bytes: &[u8],
    parents: &mut Vec<ccountry_raid_status::Row>,
    children: &mut Vec<craid_instance::Row>,
) -> (Outcome, Outcome) {
    let walk = run(
        &SliceMemory { base: 0, bytes },
        parents,
        children,
        &mut vec![0; 8],
        &mut vec![0; 8],
    );
    (walk.parents, walk.instances)
}

fn run(
    memory: &impl Memory,
    parents: &mut Vec<ccountry_raid_status::Row>,
    instances: &mut Vec<craid_instance::Row>,
    type_tokens: &mut Vec<u32>,
    show_for: &mut Vec<u32>,
) -> Walk {
    walk(
        memory,
        0,
        GS,
        Buffers {
            parents,
            type_tokens,
            instances,
            show_for,
            cooldowns: &mut Vec::new(),
            type_memo: &mut ccountry_raid_status::TypeTokens::with_definitions(8),
        },
    )
}

#[test]
fn native_values_aliases_and_dummy_parents_survive() {
    let bytes = fixture();
    let mut parents = vec![ccountry_raid_status::Row::default(); 2];
    let mut children = vec![craid_instance::Row::default(); 1];
    let (mut tokens, mut shown) = (vec![0; 3], vec![0; 2]);
    let result = run(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        &mut parents,
        &mut children,
        &mut tokens,
        &mut shown,
    );
    assert_eq!(
        (result.parents, result.instances),
        (Outcome::Rows { len: 2 }, Outcome::Rows { len: 1 })
    );
    assert_eq!(parents[0].country, -1);
    assert_eq!(parents[1].country, 3);
    assert_eq!(parents[1].canonical_slot, 1);
    assert_eq!((parents[1].eligible_start, parents[1].eligible_len), (0, 2));
    assert_eq!((parents[1].used_start, parents[1].used_len), (2, 1));
    assert_eq!(tokens, [0x4242; 3]);
    assert_eq!(
        (children[0].show_for_start, children[0].show_for_len),
        (0, 2)
    );
    assert_eq!(shown, [3, 9]);
    assert_eq!(children[0].key(), (84_u64 << 32) | u64::from(u32::MAX));
    assert_eq!(children[0].source.country, 1);
    assert_eq!(children[0].source.kind, Some(5));
    assert_eq!(children[0].source.province, 450);
    assert_eq!(children[0].source.ship_reference_type, u32::MAX);
    assert_eq!(children[0].source.ship_reference_id, 0x8765_4321);
    assert_eq!(children[0].travelled, 2_210_000);
    assert_eq!(children[0].preparation_hours, 360);
    assert_eq!(
        (
            children[0].source.building_type,
            children[0].source.building_location,
            children[0].source.building_country
        ),
        (77, 7301, -1)
    );
}

#[test]
fn absent_source_kind_does_not_read_its_payload() {
    struct WithoutKind<'a> {
        bytes: &'a [u8],
    }
    impl Memory for WithoutKind<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let kind = INSTANCE + craid_instance::SOURCE + craid_source::KIND;
            if address < kind + 4 && address.saturating_add(destination.len()) > kind {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    let mut bytes = fixture();
    put(
        &mut bytes,
        INSTANCE + craid_instance::SOURCE + craid_source::KIND_PRESENT,
        &[0],
    );
    let mut parents = vec![ccountry_raid_status::Row::default(); 2];
    let mut children = vec![craid_instance::Row::default(); 1];
    let result = run(
        &WithoutKind { bytes: &bytes },
        &mut parents,
        &mut children,
        &mut vec![0; 3],
        &mut vec![0; 2],
    );
    assert_eq!(
        (result.parents, result.instances),
        (Outcome::Rows { len: 2 }, Outcome::Rows { len: 1 })
    );
    assert_eq!(children[0].source.kind, None);
    assert_eq!(children[0].source.kind_present, 0);
    assert_eq!(children[0].source.province, 450);
}

#[test]
fn bad_children_do_not_erase_readable_parent_state() {
    for (offset, value) in [
        (craid_instance::PHASE, 6_u32),
        (craid_instance::OUTCOME, 6),
        (craid_instance::REFERENCE_TYPE, 83),
        (craid_instance::ID, 0),
        (craid_instance::DETECTED, 2),
        (craid_instance::SOURCE + craid_source::KIND_PRESENT, 2),
        (craid_instance::SOURCE + craid_source::KIND, 6),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, INSTANCE + offset, &value.to_le_bytes());
        let mut parents = vec![ccountry_raid_status::Row::default(); 2];
        let mut children = vec![craid_instance::Row::default(); 1];
        let (parent, child) = read_fixture(&bytes, &mut parents, &mut children);
        assert_eq!(parent, Outcome::Rows { len: 2 });
        assert!(
            matches!(child, Outcome::Unavailable { .. }),
            "offset {offset:x}"
        );
    }
}

#[test]
fn wrong_parent_slot_duplicate_reference_and_invalid_geometry_are_refused() {
    let mut parents = vec![ccountry_raid_status::Row::default(); 2];
    let mut children = vec![craid_instance::Row::default(); 2];
    let mut bytes = fixture();
    put(
        &mut bytes,
        PARENTS + ccountry_raid_status::COUNTRY,
        &3_i32.to_le_bytes(),
    );
    assert!(matches!(
        read_fixture(&bytes, &mut parents, &mut children).0,
        Outcome::Unavailable {
            reason: "raid parent country differs from canonical slot"
        }
    ));
    let mut bytes = fixture();
    header(
        &mut bytes,
        PARENTS + ccountry_raid_status::STRIDE + ccountry_raid_status::INSTANCES,
        CHILDREN,
        2,
    );
    put(&mut bytes, CHILDREN + 8, &(INSTANCE as u64).to_le_bytes());
    assert_eq!(
        read_fixture(&bytes, &mut parents, &mut children).1,
        Outcome::Unavailable {
            reason: "duplicate raid instance reference"
        }
    );
    put(
        &mut bytes,
        ROOT + COUNTRY_STATUSES_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    assert!(matches!(
        read_fixture(&bytes, &mut parents, &mut children).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn authenticated_empty_root_needs_no_country_roster() {
    let mut bytes = fixture();
    header(&mut bytes, ROOT + COUNTRY_STATUSES, 0, 0);
    header(&mut bytes, GS + cgame_state::COUNTRIES, 0, 0);
    assert_eq!(
        read_fixture(&bytes, &mut Vec::new(), &mut Vec::new()),
        (Outcome::Rows { len: 0 }, Outcome::Rows { len: 0 })
    );
    put(&mut bytes, ROOT, &0_u64.to_le_bytes());
    assert!(matches!(
        read_fixture(&bytes, &mut Vec::new(), &mut Vec::new()).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn cooldowns_keep_owner_order_and_type_and_grow_with_demand() {
    const COOLDOWNS: usize = 0x8b00;
    let mut bytes = fixture();
    let parent = PARENTS + ccountry_raid_status::STRIDE;
    header(
        &mut bytes,
        parent + ccountry_raid_status::TARGET_COOLDOWNS,
        COOLDOWNS,
        2,
    );
    for index in 0..2 {
        let entry = COOLDOWNS + index * ccountry_raid_status::TARGET_COOLDOWN_STRIDE;
        put(
            &mut bytes,
            entry,
            &(ccountry_raid_status::COOLDOWN_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            entry + ccountry_raid_status::COOLDOWN_TYPE,
            &(DEFINITION as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            entry + ccountry_raid_status::COOLDOWN_SCOPE + craid_target::PROVINCE_ARM,
            &(PROVINCE as u64).to_le_bytes(),
        );
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut cooldowns = vec![CooldownRow::default(); 1];
    let result = walk(
        &memory,
        0,
        GS,
        Buffers {
            parents: &mut vec![ccountry_raid_status::Row::default(); 2],
            type_tokens: &mut vec![0; 8],
            instances: &mut vec![craid_instance::Row::default(); 1],
            show_for: &mut vec![0; 8],
            cooldowns: &mut cooldowns,
            type_memo: &mut ccountry_raid_status::TypeTokens::with_definitions(8),
        },
    );
    assert_eq!(result.cooldowns, Outcome::Rows { len: 2 });
    assert_eq!(
        (
            cooldowns[0].owner_slot,
            cooldowns[0].ordinal,
            cooldowns[0].raid_type
        ),
        (1, 0, 0x4242)
    );
    assert_eq!(cooldowns[0].scope.province, 450);
}

fn scaled(countries: usize) -> Vec<u8> {
    let slots = countries + 1;
    let (parents, array, links) = (0x10000, 0x10000 + slots * ccountry_raid_status::STRIDE, 0);
    let array = array.max(parents + slots * ccountry_raid_status::STRIDE);
    let links = links.max(array + slots * 8);
    let objects = links + slots * 4;
    const PER: usize = 0x400;
    let mut bytes = vec![0_u8; objects + slots * PER];
    put(
        &mut bytes,
        GS + cgame_state::RAID_SYSTEM,
        &(ROOT as u64).to_le_bytes(),
    );
    put(&mut bytes, ROOT, &(VTABLE_RVA.0 as u64).to_le_bytes());
    header(&mut bytes, ROOT + COUNTRY_STATUSES, parents, slots as u32);
    header(&mut bytes, GS + cgame_state::COUNTRIES, array, slots as u32);
    header(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        links,
        slots as u32,
    );
    put(
        &mut bytes,
        DEFINITION,
        &(crate::raid_database::TYPE_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, DEFINITION + 8, &0x4242_u32.to_le_bytes());
    put(
        &mut bytes,
        PROVINCE,
        &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, PROVINCE + cprovince::ID, &450_u32.to_le_bytes());
    for slot in 0..slots {
        let parent = parents + slot * ccountry_raid_status::STRIDE;
        put(
            &mut bytes,
            parent,
            &(ccountry_raid_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        if slot == 0 {
            put(
                &mut bytes,
                parent + ccountry_raid_status::COUNTRY,
                &(-1_i32).to_le_bytes(),
            );
            continue;
        }
        let base = objects + slot * PER;
        let (country, types, instance, shown, cooldowns) =
            (base, base + 0x40, base + 0x100, base + 0x280, base + 0x300);
        put(
            &mut bytes,
            array + slot * 8,
            &(country as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(&mut bytes, links + slot * 4, &(slot as u32).to_le_bytes());
        put(
            &mut bytes,
            parent + ccountry_raid_status::COUNTRY,
            &(slot as i32).to_le_bytes(),
        );
        header(
            &mut bytes,
            parent + ccountry_raid_status::ELIGIBLE_TYPES,
            types,
            8,
        );
        header(
            &mut bytes,
            parent + ccountry_raid_status::USED_TYPES,
            types,
            1,
        );
        for index in 0..8 {
            put(
                &mut bytes,
                types + index * 8,
                &(DEFINITION as u64).to_le_bytes(),
            );
        }
        header(
            &mut bytes,
            parent + ccountry_raid_status::INSTANCES,
            base + 0x80,
            1,
        );
        put(&mut bytes, base + 0x80, &(instance as u64).to_le_bytes());
        put(
            &mut bytes,
            instance,
            &(craid_instance::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            instance + craid_instance::REFERENCE_TYPE,
            &84_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            instance + craid_instance::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            instance + craid_instance::PHASE,
            &3_i32.to_le_bytes(),
        );
        let source = instance + craid_instance::SOURCE;
        put(
            &mut bytes,
            source + craid_source::KIND,
            &5_u32.to_le_bytes(),
        );
        put(&mut bytes, source + craid_source::KIND_PRESENT, &[1]);
        put(
            &mut bytes,
            source + craid_source::PROVINCE,
            &(PROVINCE as u64).to_le_bytes(),
        );
        header(
            &mut bytes,
            instance + craid_instance::SHOW_FOR_COUNTRIES,
            shown,
            2,
        );
        put(&mut bytes, shown, &1_u32.to_le_bytes());
        put(&mut bytes, shown + 4, &(slot as u32).to_le_bytes());
        header(
            &mut bytes,
            parent + ccountry_raid_status::TARGET_COOLDOWNS,
            cooldowns,
            2,
        );
        for index in 0..2 {
            let entry = cooldowns + index * ccountry_raid_status::TARGET_COOLDOWN_STRIDE;
            put(
                &mut bytes,
                entry,
                &(ccountry_raid_status::COOLDOWN_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + ccountry_raid_status::COOLDOWN_TYPE,
                &(DEFINITION as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + ccountry_raid_status::COOLDOWN_SCOPE + craid_target::PROVINCE_ARM,
                &(PROVINCE as u64).to_le_bytes(),
            );
        }
    }
    bytes
}

#[test]
fn a_first_hour_past_the_default_type_token_arena_grows_to_its_demand() {
    let countries = 600;
    let bytes = scaled(countries);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let demand = required_type_tokens(&memory, 0, GS).unwrap();
    assert!(demand > 4096, "{demand}");
    let slots = countries + 1;
    let run = |tokens: usize| {
        let mut parents = vec![ccountry_raid_status::Row::default(); slots];
        let mut instances = vec![craid_instance::Row::default(); slots];
        let mut tokens = vec![0; tokens];
        let mut shown = vec![0; slots * 2];
        let mut cooldowns = vec![CooldownRow::default(); slots * 2];
        walk(
            &memory,
            0,
            GS,
            Buffers {
                parents: &mut parents,
                type_tokens: &mut tokens,
                instances: &mut instances,
                show_for: &mut shown,
                cooldowns: &mut cooldowns,
                type_memo: &mut ccountry_raid_status::TypeTokens::with_definitions(64),
            },
        )
    };
    let grown = run(4096);
    assert_eq!(grown.parents, Outcome::Rows { len: slots });
    assert_eq!(grown.required_type_tokens, demand);
    let sized = run(demand);
    assert_eq!(sized.parents, Outcome::Rows { len: slots });
    assert_eq!(sized.required_type_tokens, demand);
}

