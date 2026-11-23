use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::Cell;

const ACCOUNT: usize = 0x1000;
const INFLUENCE_DATA: usize = 0x2000;
const PROJECTION_DATA: usize = 0x2100;
const UPGRADE_NODES: usize = 0x2200;
const UPGRADE_DEFINITION: usize = 0x2300;
const GOAL_LIST: usize = 0x2400;
const GOAL_DEFINITION: usize = 0x2500;

fn put(bytes: &mut [u8], address: usize, value: impl AsRef<[u8]>) {
    let value = value.as_ref();
    bytes[address..address + value.len()].copy_from_slice(value);
}
fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, (data as u64).to_le_bytes());
    put(bytes, address + 8, count.to_le_bytes());
    put(bytes, address + 12, count.to_le_bytes());
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x3000];
    put(&mut bytes, ACCOUNT + COUNTRY, 4999_u32.to_le_bytes());
    array(&mut bytes, ACCOUNT + INFLUENCE, INFLUENCE_DATA, 5);
    array(&mut bytes, ACCOUNT + POWER_PROJECTION, PROJECTION_DATA, 4);
    for (address, values) in [
        (INFLUENCE_DATA, &[i64::MIN, -1, 0, 100001, i64::MAX][..]),
        (PROJECTION_DATA, &[3, -7, 99, 1][..]),
    ] {
        for (ordinal, value) in values.iter().enumerate() {
            put(&mut bytes, address + ordinal * 8, value.to_le_bytes());
        }
    }
    for (offset, value) in [
        (INFLUENCE_SHARE, -5_i64),
        (POWER_PROJECTION_SHARE, 100001),
        (INITIATIVE, 17),
        (CONTRIBUTION, 31),
        (CONTRIBUTION_GAIN, -9),
        (CONTRIBUTION_GAIN + 8, 123456),
        (PREVIOUS_WAR_SCORE, 877),
    ] {
        put(&mut bytes, ACCOUNT + offset, value.to_le_bytes());
    }
    let table = ACCOUNT + UPGRADES;
    put(&mut bytes, table + 8, (UPGRADE_NODES as u64).to_le_bytes());
    put(&mut bytes, table + 0x10, 1_u32.to_le_bytes());
    put(&mut bytes, table + 0x14, 7_u32.to_le_bytes());
    bytes[table + 0x18] = 5;
    bytes[UPGRADE_NODES + 4] = 1;
    put(&mut bytes, UPGRADE_NODES + 8, 901_u32.to_le_bytes());
    put(
        &mut bytes,
        UPGRADE_NODES + 16,
        (UPGRADE_DEFINITION as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        UPGRADE_DEFINITION,
        (UPGRADE_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, UPGRADE_DEFINITION + 8, 1123_u32.to_le_bytes());
    array(&mut bytes, ACCOUNT + COMPLETED_GOALS, GOAL_LIST, 3);
    put(
        &mut bytes,
        GOAL_LIST,
        (GOAL_DEFINITION as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        GOAL_LIST + 16,
        (GOAL_DEFINITION as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        GOAL_DEFINITION,
        (crate::cfaction_goal_status::DEFINITION_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, GOAL_DEFINITION + 8, 888_u32.to_le_bytes());
    bytes
}

#[test]
fn faction_member_account_retains_independent_balances_sources_and_goal_occurrences() {
    let bytes = fixture();
    let mut output = Snapshot::default();
    read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        Array {
            data: ACCOUNT,
            count: 1,
        },
        0,
        &mut output,
    )
    .unwrap();
    assert_eq!(output.country, 4999);
    assert_eq!(output.influence, [i64::MIN, -1, 0, 100001, i64::MAX]);
    assert_eq!(output.power_projection, [3, -7, 99, 1]);
    assert_eq!(
        (
            output.influence_share_raw,
            output.power_projection_share_raw
        ),
        (-5, 100001)
    );
    assert_eq!((output.initiative_raw, output.contribution_raw), (17, 31));
    assert_eq!(output.contribution_gain, [-9, 123456]);
    assert_eq!(output.previous_war_score_raw, 877);
    assert_eq!(
        output.upgrades,
        [Upgrade {
            group: 901,
            token: 1123
        }]
    );
    assert_eq!(output.completed_goals, [Some(888), None, Some(888)]);
}

struct Counted<'a> {
    bytes: &'a [u8],
    fault: usize,
    hits: Cell<usize>,
    reads: Cell<usize>,
}
impl Memory for Counted<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.set(self.reads.get() + 1);
        if address <= self.fault && self.fault < address + output.len() {
            self.hits.set(self.hits.get() + 1);
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

#[test]
fn faction_member_account_refuses_each_independent_fault_without_retry() {
    let bytes = fixture();
    for fault in [
        ACCOUNT + COUNTRY,
        ACCOUNT + INFLUENCE,
        PROJECTION_DATA + 8,
        ACCOUNT + INITIATIVE,
        ACCOUNT + CONTRIBUTION_GAIN,
        UPGRADE_NODES + 4,
        UPGRADE_DEFINITION,
        GOAL_LIST + 8,
    ] {
        let memory = Counted {
            bytes: &bytes,
            fault,
            hits: Cell::new(0),
            reads: Cell::new(0),
        };
        assert!(
            read_into(
                &memory,
                0,
                Array {
                    data: ACCOUNT,
                    count: 1
                },
                0,
                &mut Snapshot::default()
            )
            .is_err()
        );
        assert_eq!(memory.hits.get(), 1);
    }
}

#[test]
fn faction_member_account_demand_is_one_selected_account() {
    let initial = fixture();
    let mut bytes = initial.clone();
    let accounts = bytes.len();
    bytes.resize(accounts + 4 * STRIDE, 0);
    let selected = 2;
    let address = accounts + selected * STRIDE;
    bytes[address..address + STRIDE].copy_from_slice(&initial[ACCOUNT..ACCOUNT + STRIDE]);
    let memory = Counted {
        bytes: &bytes,
        fault: accounts,
        hits: Cell::new(0),
        reads: Cell::new(0),
    };
    let mut output = Snapshot::default();
    let source = Array {
        data: accounts,
        count: 4,
    };
    read_into(&memory, 0, source, selected as u32, &mut output).unwrap();
    assert_eq!(output.country, 4999);
    assert_eq!(memory.hits.get(), 0);
    let pointers = (
        output.influence.as_ptr(),
        output.power_projection.as_ptr(),
        output.upgrades.as_ptr(),
        output.completed_goals.as_ptr(),
    );
    read_into(&memory, 0, source, selected as u32, &mut output).unwrap();
    assert_eq!(
        pointers,
        (
            output.influence.as_ptr(),
            output.power_projection.as_ptr(),
            output.upgrades.as_ptr(),
            output.completed_goals.as_ptr()
        )
    );
    let reads = memory.reads.get();
    assert!(read_into(&memory, 0, source, 4, &mut output).is_err());
    assert_eq!(
        memory.reads.get(),
        reads,
        "out-of-demand indices acquire nothing"
    );
}

#[test]
fn projection_coefficients_preserve_loaded_mod_values_without_default_substitution() {
    struct Factors;
    impl Memory for Factors {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let Some(index) = PROJECTION_FACTORS.iter().position(|&offset| offset.0 == at) else {
                return false;
            };
            if out.len() != 8 {
                return false;
            }
            out.copy_from_slice(&(-17i64 + index as i64).to_le_bytes());
            true
        }
    }
    assert_eq!(
        projection_factors(&Factors, 0).unwrap(),
        [-17, -16, -15, -14, -13, -12, -11]
    );
}
