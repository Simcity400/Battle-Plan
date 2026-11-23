use engine_native_memory::{Memory, memory::SliceMemory};
use crate::{
    cdoctrine_folder_template as folder,
    cdoctrine_sharing_status as doctrine,
    cfaction_program_status as programs,
    cfaction_rule_status as rules,
    ctechnology_sharing_faction as research,
};
use engine_native::cfaction;

const FACTION: usize = 0x1000;
const DATA: usize = 0x3000;

fn put(bytes: &mut [u8], address: usize, value: impl AsRef<[u8]>) {
    let value = value.as_ref();
    bytes[address..address + value.len()].copy_from_slice(value);
}
fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, (data as u64).to_le_bytes());
    put(bytes, address + 8, count.to_le_bytes());
    put(bytes, address + 12, count.to_le_bytes());
}

#[test]
fn faction_companions_keep_programme_handles_and_rules_in_native_order() {
    let mut bytes = vec![0; 0x5000];
    let status = FACTION + programs::STATUS;
    put(
        &mut bytes,
        status,
        (programs::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(&mut bytes, status + programs::PROGRAMS, DATA, 4);
    let expected = [0_u64, 0x8000_ffff_1234_5678, 17, 17];
    for (ordinal, value) in expected.into_iter().enumerate() {
        put(&mut bytes, DATA + ordinal * 8, value.to_le_bytes());
    }
    let mut references = Vec::new();
    programs::read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut references,
    )
    .unwrap();
    assert_eq!(references, expected);

    let status = FACTION + rules::STATUS;
    put(&mut bytes, status, (rules::VTABLE_RVA.0 as u64).to_le_bytes());
    put(
        &mut bytes,
        status + rules::OWNER,
        (FACTION as u64).to_le_bytes(),
    );
    array(&mut bytes, status + rules::RULES, DATA + 0x100, 3);
    for ordinal in 0..3 {
        put(
            &mut bytes,
            DATA + 0x100 + ordinal * 8,
            ((DATA + 0x200) as u64).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        DATA + 0x200,
        (rules::DEFINITION_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DATA + 0x200 + rules::DEFINITION_TOKEN,
        49321_u32.to_le_bytes(),
    );
    let mut selected = Vec::new();
    rules::read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut selected,
    )
    .unwrap();
    assert_eq!(selected, [49321; 3]);
    put(&mut bytes, status + rules::OWNER, 99_u64.to_le_bytes());
    assert!(
        rules::read_into(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            FACTION,
            &mut selected
        )
        .is_err()
    );
}

#[test]
fn faction_companions_retain_sharing_bonuses_country_aliases_and_doctrine_folder_order() {
    let mut bytes = vec![0; 0x5000];
    let status = 0x2000;
    put(
        &mut bytes,
        status,
        (research::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        status + research::OWNER,
        (FACTION as u64).to_le_bytes(),
    );
    array(&mut bytes, status + research::COUNTRIES, DATA, 3);
    put(
        &mut bytes,
        status + research::BASE,
        (-131_i64).to_le_bytes(),
    );
    array(&mut bytes, status + research::BONUSES, DATA + 0x100, 2);
    for (ordinal, country) in [0_u32, 4999, 4999].into_iter().enumerate() {
        put(&mut bytes, DATA + ordinal * 4, country.to_le_bytes());
    }
    for (ordinal, value) in [i64::MIN, i64::MAX].into_iter().enumerate() {
        put(&mut bytes, DATA + 0x100 + ordinal * 8, value.to_le_bytes());
    }
    let mut output = research::Snapshot::default();
    research::read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        status,
        &mut output,
    )
    .unwrap();
    assert_eq!(output.countries, [0, 4999, 4999]);
    assert_eq!(output.base_raw, -131);
    assert_eq!(output.bonuses, [i64::MIN, i64::MAX]);
    put(&mut bytes, status + research::OWNER, 1_u64.to_le_bytes());
    assert!(
        research::read_into(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            FACTION,
            status,
            &mut output
        )
        .is_err()
    );

    let status = 0x2400;
    put(
        &mut bytes,
        status,
        (doctrine::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(&mut bytes, status + doctrine::FOLDERS, DATA + 0x200, 3);
    for ordinal in 0..3 {
        put(
            &mut bytes,
            DATA + 0x200 + ordinal * 8,
            ((DATA + 0x300) as u64).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        DATA + 0x300,
        (folder::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DATA + 0x300 + folder::TOKEN,
        45454_u32.to_le_bytes(),
    );
    let mut selected = Vec::new();
    doctrine::read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        status,
        &mut selected,
    )
    .unwrap();
    assert_eq!(selected, [45454; 3]);
}

#[test]
fn faction_companions_preserve_signed_manpower_contributions() {
    let mut bytes = vec![0; 0x5000];
    let pool = FACTION + cfaction::MANPOWER_POOL;
    put(
        &mut bytes,
        pool,
        (engine_native_parts::carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        pool + engine_native_parts::carmy_manpower_values::DATA,
        DATA,
        3,
    );
    let expected = [
        cfaction::ManpowerContribution {
            country: 0,
            value: -7,
        },
        cfaction::ManpowerContribution {
            country: u32::MAX,
            value: i32::MAX,
        },
        cfaction::ManpowerContribution {
            country: 0,
            value: 0,
        },
    ];
    for (ordinal, entry) in expected.into_iter().enumerate() {
        put(&mut bytes, DATA + ordinal * 8, entry.country.to_le_bytes());
        put(
            &mut bytes,
            DATA + ordinal * 8 + 4,
            entry.value.to_le_bytes(),
        );
    }
    let mut output = Vec::new();
    cfaction::manpower_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut output,
    )
    .unwrap();
    assert_eq!(output, expected);
}

#[test]
fn faction_companions_allow_empty_arrays_without_reading_a_data_pointer() {
    struct NoPayload(Vec<u8>);
    impl Memory for NoPayload {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert!(address < DATA);
            SliceMemory {
                base: 0,
                bytes: &self.0,
            }
            .copy(address, output)
        }
    }
    let mut bytes = vec![0; DATA];
    for (offset, vtable, array_offset) in [
        (programs::STATUS, programs::VTABLE_RVA, programs::PROGRAMS),
        (rules::STATUS, rules::VTABLE_RVA, rules::RULES),
    ] {
        let status = FACTION + offset;
        put(&mut bytes, status, (vtable.0 as u64).to_le_bytes());
        array(&mut bytes, status + array_offset, usize::MAX, 0);
    }
    put(
        &mut bytes,
        FACTION + rules::STATUS + rules::OWNER,
        (FACTION as u64).to_le_bytes(),
    );
    let memory = NoPayload(bytes);
    programs::read_into(&memory, 0, FACTION, &mut Vec::new()).unwrap();
    rules::read_into(&memory, 0, FACTION, &mut Vec::new()).unwrap();
}
