use super::*;
use engine_native_parts::carmy_manpower_values;
use engine_native::{
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cgame_state,
    cname_group_member,
    creferenced_division_template,
};
use crate::cmilitary_deployment as deployment;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x1000;
const COUNTRY: usize = 0x3000;
const STATUS: usize = 0x4000;
const QUEUE: usize = 0x4100;
const CONVEYOR: usize = 0x5000;
const LINE_ARRAY: usize = 0x5200;
const LINE: usize = 0x6000;
const NAME_OBJECT: usize = 0x6200;
const DEPLOYMENT: usize = 0x7000;
const EQUIPMENT: usize = 0x7400;
const HAVE: usize = 0x7500;
const NEED: usize = 0x7580;
const TEMPLATE: usize = 0x7600;
const ALLOCATIONS: usize = 0x7680;
const VARIANT: usize = 0x7700;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
}
fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
    put(bytes, header, &(data as u64).to_le_bytes());
    put(bytes, header + 8, &count.to_le_bytes());
    put(bytes, header + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x8000];
    array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x2000, 3);
    array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x2100, 3);
    for slot in [1, 2] {
        put(
            &mut bytes,
            0x2000 + slot * 8,
            &(COUNTRY as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x2100 + slot * 4, &1_u32.to_le_bytes());
    }
    put(
        &mut bytes,
        COUNTRY,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::ID, &2_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY + ccountry::DEPLOYMENT_MANAGER,
        &(STATUS as u64).to_le_bytes(),
    );
    put(&mut bytes, STATUS, &(VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, STATUS + OWNER, &(COUNTRY as u64).to_le_bytes());
    array(&mut bytes, STATUS + CONVEYORS, QUEUE, 1);
    put(&mut bytes, QUEUE, &(CONVEYOR as u64).to_le_bytes());

    put(
        &mut bytes,
        CONVEYOR,
        &(cmilitary_deployment_conveyor::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::TYPE,
        &0xfedc_ba98_0000_0040_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::TEMPLATE,
        &(TEMPLATE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::NAME,
        b"Panzer\0",
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::NAME + engine_native_memory::msvc_string::LENGTH,
        &6_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::NAME + engine_native_memory::msvc_string::CAPACITY,
        &15_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::AMOUNT,
        &(-3_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::LOCATION,
        &u32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::PRIORITY,
        &7_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::ROLE_TOKEN,
        &0_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::CLOSED,
        &[0xaa],
    );
    array(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::LINES,
        LINE_ARRAY,
        1,
    );
    put(&mut bytes, LINE_ARRAY, &(LINE as u64).to_le_bytes());

    put(
        &mut bytes,
        TEMPLATE,
        &(creferenced_division_template::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE + creferenced_division_template::TYPE,
        &0x0000_0021_0000_0034_u64.to_le_bytes(),
    );

    put(
        &mut bytes,
        LINE,
        &(cmilitary_deployment_line::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LINE + cmilitary_deployment_line::TYPE,
        &0x0000_00c8_0000_003f_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        LINE + cmilitary_deployment_line::NAME,
        &(NAME_OBJECT as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LINE + cmilitary_deployment_line::AMOUNT,
        &(-1_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        LINE + cmilitary_deployment_line::DEPLOYMENT,
        &(DEPLOYMENT as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LINE + cmilitary_deployment_line::PARENT,
        &(CONVEYOR as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        NAME_OBJECT,
        &(cname_group_member::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        NAME_OBJECT + cname_group_member::ORDER,
        &i32::MIN.to_le_bytes(),
    );
    put(
        &mut bytes,
        NAME_OBJECT + cname_group_member::OVERRIDE_NAME + engine_native_memory::msvc_string::LENGTH,
        &0_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        NAME_OBJECT + cname_group_member::OVERRIDE_NAME + engine_native_memory::msvc_string::CAPACITY,
        &engine_native_memory::msvc_string::INLINE_CAPACITY.to_le_bytes(),
    );
    put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_A, &[1]);
    put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_B, &[0]);

    put(
        &mut bytes,
        DEPLOYMENT,
        &(deployment::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::SECONDARY_VTABLE,
        &(deployment::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::EQUIPMENT_POOL,
        &(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        DEPLOYMENT + deployment::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        EQUIPMENT,
        1,
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::EQUIPMENT_POOL + cequipment_variant_pool::ALLOW_ZERO,
        &[1],
    );
    array(
        &mut bytes,
        DEPLOYMENT + deployment::EQUIPMENT_POOL + cequipment_variant_pool::ALLOCATIONS,
        ALLOCATIONS,
        1,
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::DISTRIBUTABLE_PRIORITY,
        &i32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::HALTING_REASON,
        &i32::MIN.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::HALTING_REASON_PRESENT,
        &[0xaa],
    );
    array(
        &mut bytes,
        DEPLOYMENT + deployment::EQUIPMENT_POOL + cequipment_variant_pool::ALLOCATIONS,
        ALLOCATIONS,
        1,
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::DISTRIBUTABLE_PRIORITY,
        &i32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::HALTING_REASON,
        &i32::MIN.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::HALTING_REASON_PRESENT,
        &[0xaa],
    );
    put(&mut bytes, EQUIPMENT, &(VARIANT as u64).to_le_bytes());
    put(&mut bytes, EQUIPMENT + 8, &i64::MIN.to_le_bytes());
    put(
        &mut bytes,
        VARIANT,
        &(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        VARIANT + cequipment_variant::TYPE,
        &0x1234_5678_0000_0046_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::TRAINING,
        &0_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::MAX_TRAINING,
        &98_799_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::MAX_MANPOWER,
        &i32::MIN.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + deployment::OWNING_LINE,
        &(LINE as u64).to_le_bytes(),
    );
    for (offset, container, count) in [
        (deployment::MANPOWER_HAVE, HAVE, 2_u32),
        (deployment::MANPOWER_NEED, NEED, 0),
    ] {
        put(
            &mut bytes,
            DEPLOYMENT + offset,
            &(carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            DEPLOYMENT + offset + carmy_manpower_values::DATA,
            container,
            count,
        );
    }
    put(&mut bytes, HAVE, &0x8000_0001_u32.to_le_bytes());
    put(&mut bytes, HAVE + 4, &(-5_i32).to_le_bytes());
    put(&mut bytes, HAVE + 8, &0_u32.to_le_bytes());
    put(&mut bytes, HAVE + 12, &i32::MAX.to_le_bytes());
    bytes
}

type Buffers = (
    Vec<ConveyorRow>,
    Vec<LineRow>,
    Vec<ManpowerRow>,
    Vec<EquipmentRow>,
    Vec<u8>,
    Vec<u8>,
);

fn buffers() -> Buffers {
    Default::default()
}

#[test]
fn queue_keeps_full_handles_native_order_zero_training_and_signed_counts() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let (mut conveyors, mut lines, mut manpower, mut equipment, mut names, mut custom) = buffers();
    let outcomes = walk(
        &memory,
        0,
        BASE,
        &mut conveyors,
        &mut lines,
        &mut manpower,
        &mut equipment,
        &mut names,
        &mut custom,
    );
    assert_eq!(outcomes.conveyors, Outcome::Rows { len: 1 });
    assert_eq!(outcomes.lines, Outcome::Rows { len: 1 });
    assert_eq!(outcomes.manpower, Outcome::Rows { len: 2 });
    assert_eq!(outcomes.equipment, Outcome::Rows { len: 1 });
    assert_eq!((outcomes.name_bytes, outcomes.required_name_bytes), (6, 6));
    assert_eq!(
        (
            outcomes.custom_name_bytes,
            outcomes.required_custom_name_bytes
        ),
        (0, 0)
    );
    assert_eq!(&names[..6], b"Panzer");
    assert_eq!(
        conveyors[0],
        ConveyorRow {
            handle: 0xfedc_ba98_0000_0040,
            country: 2,
            ordinal: 0,
            template: 0x0000_0021_0000_0034,
            amount: -3,
            location: u32::MAX,
            priority: 7,
            role_token: 0,
            closed: 0xaa,
            name_start: 0,
            name_len: 6,
            exile_country: None,
        }
    );
    assert_eq!(
        lines[0],
        LineRow {
            handle: 0x0000_00c8_0000_003f,
            conveyor: 0xfedc_ba98_0000_0040,
            country: 2,
            ordinal: 0,
            name_order: i32::MIN,
            amount: -1,
            training_raw: 0,
            max_training_raw: 98_799,
            max_manpower: i32::MIN,
            allow_zero_entries: 1,
            equipment_allocations: 1,
            distributable_priority: i32::MAX,
            halting_reason_present: 0xaa,
            halting_reason: i32::MIN,
            custom_name_start: 0,
            custom_name_len: 0,
            custom_flag_a: 1,
            custom_flag_b: 0,
        }
    );
    assert_eq!(
        &manpower[..2],
        &[
            ManpowerRow {
                line: 0x0000_00c8_0000_003f,
                side: deployment::SIDE_HAVE,
                ordinal: 0,
                source_country: 0x8000_0001,
                amount: -5,
            },
            ManpowerRow {
                line: 0x0000_00c8_0000_003f,
                side: deployment::SIDE_HAVE,
                ordinal: 1,
                source_country: 0,
                amount: i32::MAX,
            },
        ]
    );
    assert_eq!(
        equipment[0],
        EquipmentRow {
            line: 0x0000_00c8_0000_003f,
            ordinal: 0,
            variant_handle: 0x1234_5678_0000_0046,
            amount_raw: i64::MIN,
        }
    );
}

#[test]
fn a_deployment_pointing_at_another_object_is_refused_like_a_null_one() {
    for value in [0_u64, CONVEYOR as u64, LINE as u64 + 8, NAME_OBJECT as u64] {
        let mut bytes = fixture();
        put(
            &mut bytes,
            DEPLOYMENT + deployment::OWNING_LINE,
            &value.to_le_bytes(),
        );
        let (mut conveyors, mut lines, mut manpower, mut equipment, mut names, mut custom) =
            buffers();
        let outcomes = walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            BASE,
            &mut conveyors,
            &mut lines,
            &mut manpower,
            &mut equipment,
            &mut names,
            &mut custom,
        );
        let reason = Outcome::Unavailable {
            reason: "military deployment owning-line back pointer mismatch",
        };
        assert_eq!(outcomes.lines, reason, "back pointer {value:#x}");
        assert_eq!(outcomes.conveyors, reason, "back pointer {value:#x}");
    }
    let bytes = fixture();
    let (mut conveyors, mut lines, mut manpower, mut equipment, mut names, mut custom) = buffers();
    let outcomes = walk(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        0,
        BASE,
        &mut conveyors,
        &mut lines,
        &mut manpower,
        &mut equipment,
        &mut names,
        &mut custom,
    );
    assert_eq!(outcomes.lines, Outcome::Rows { len: 1 });
}

#[test]
fn every_broken_link_is_unavailable_rather_than_a_shorter_queue() {
    for (address, reason) in [
        (
            COUNTRY + ccountry::DEPLOYMENT_MANAGER,
            "deployment status pointer unreadable or null",
        ),
        (STATUS, "deployment status vtable mismatch"),
        (STATUS + OWNER, "deployment status country mismatch"),
        (QUEUE, "deployment conveyor pointer unreadable or null"),
        (CONVEYOR, "deployment conveyor vtable mismatch"),
        (
            CONVEYOR + cmilitary_deployment_conveyor::TEMPLATE,
            "deployment conveyor template pointer unreadable or null",
        ),
        (LINE_ARRAY, "deployment line pointer unreadable or null"),
        (LINE, "deployment line vtable mismatch"),
        (
            LINE + cmilitary_deployment_line::PARENT,
            "deployment line parent conveyor mismatch",
        ),
        (
            LINE + cmilitary_deployment_line::NAME,
            "deployment line name pointer unreadable or null",
        ),
        (
            LINE + cmilitary_deployment_line::DEPLOYMENT,
            "deployment line deployment pointer unreadable or null",
        ),
        (DEPLOYMENT, "military deployment vtable mismatch"),
        (
            DEPLOYMENT + deployment::SECONDARY_VTABLE,
            "military deployment secondary vtable mismatch",
        ),
        (
            DEPLOYMENT + deployment::OWNING_LINE,
            "military deployment owning-line back pointer mismatch",
        ),
        (
            DEPLOYMENT + deployment::MANPOWER_HAVE,
            "military deployment manpower container vtable mismatch",
        ),
        (NAME_OBJECT, "division name vtable mismatch"),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, &0_u64.to_le_bytes());
        let (mut conveyors, mut lines, mut manpower, mut equipment, mut names, mut custom) =
            buffers();
        let outcomes = walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            BASE,
            &mut conveyors,
            &mut lines,
            &mut manpower,
            &mut equipment,
            &mut names,
            &mut custom,
        );
        assert_eq!(
            outcomes.conveyors,
            Outcome::Unavailable { reason },
            "{address:#x}"
        );
        assert_eq!(outcomes.lines, Outcome::Unavailable { reason });
        assert_eq!(outcomes.manpower, Outcome::Unavailable { reason });
        assert_eq!(outcomes.equipment, Outcome::Unavailable { reason });
    }
}

#[test]
fn a_proven_empty_queue_is_rows_and_a_missing_country_table_is_unavailable() {
    let mut bytes = fixture();
    array(&mut bytes, STATUS + CONVEYORS, 0, 0);
    let (mut conveyors, mut lines, mut manpower, mut equipment, mut names, mut custom) = buffers();
    let outcomes = walk(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        0,
        BASE,
        &mut conveyors,
        &mut lines,
        &mut manpower,
        &mut equipment,
        &mut names,
        &mut custom,
    );
    assert_eq!(outcomes.conveyors, Outcome::Rows { len: 0 });
    assert_eq!(outcomes.equipment, Outcome::Rows { len: 0 });

    let mut bytes = fixture();
    for slot in [1, 2] {
        put(&mut bytes, 0x2000 + slot * 8, &0_u64.to_le_bytes());
    }
    let outcomes = walk(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        0,
        BASE,
        &mut conveyors,
        &mut lines,
        &mut manpower,
        &mut equipment,
        &mut names,
        &mut custom,
    );
    assert_eq!(
        outcomes.conveyors,
        Outcome::Unavailable {
            reason: "country table has no readable countries"
        }
    );
}

#[test]
fn template_configuration_authenticates_manager_and_retains_raw_selection_and_lock() {
    let mut bytes = fixture();
    let handle = (u64::from(u32::MAX) << 32) | 52;
    put(
        &mut bytes,
        STATUS + SELECTED_STAFF_TEMPLATE,
        &handle.to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::LOCK_ALL_TEMPLATES, &[7]);
    put(&mut bytes, STATUS + NEXT_HQ_ORDER, &(-19_i32).to_le_bytes());
    let country = engine_native_memory::country_identity::Country {
        pointer: COUNTRY,
        id: 2,
        canonical_slot: 1,
    };
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    assert_eq!(
        template_state(&memory, 0, country).unwrap(),
        TemplateState {
            manager: STATUS,
            country: 2,
            selected_staff: handle,
            next_hq_order: -19,
            lock_all: 7
        }
    );
    put(&mut bytes, STATUS + OWNER, &0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    assert_eq!(
        template_state(&memory, 0, country),
        Err("deployment status country mismatch")
    );
}
