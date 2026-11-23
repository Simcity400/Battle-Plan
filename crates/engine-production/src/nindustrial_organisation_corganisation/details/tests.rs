use super::*;
use engine_native_memory::memory::SliceMemory;
const ORG: usize = 0x1000;
const HEAD: usize = 0x2000;
const QUEUE: usize = 0x3000;
fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put(bytes, at, &(data as u64).to_le_bytes());
    put(bytes, at + 8, &count.to_le_bytes());
    put(bytes, at + 12, &count.to_le_bytes());
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x6000];
    put(&mut bytes, ORG + UNLOCKED, &(HEAD as u64).to_le_bytes());
    for offset in [0, 8, 16] {
        put(&mut bytes, HEAD + offset, &(HEAD as u64).to_le_bytes());
    }
    bytes[HEAD + 0x19] = 1;
    put(
        &mut bytes,
        ORG + VARIABLES,
        &(cvariables::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ORG + VARIABLES + cvariables::STORAGE,
        &(cvariables::EMPTY_STORAGE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ORG + VARIABLES + cvariables::SEED,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        ORG + FLAGS,
        &(cflag_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    bytes
}
fn run(bytes: &[u8]) -> Result<Row, &'static str> {
    let mut row = Row::default();
    capture(
        &SliceMemory { base: 0, bytes },
        0,
        ORG,
        (17_u64 << 32) | 79,
        5,
        &mut row,
        &mut Scratch::default(),
    )?;
    Ok(row)
}
#[test]
fn planned_traits_keep_native_order_repeats_cursor_and_unspent_points() {
    let mut bytes = fixture();
    array(&mut bytes, ORG + QUEUED, QUEUE, 4);
    for (index, token) in [77_u32, 42, 77, u32::MAX].iter().enumerate() {
        put(&mut bytes, QUEUE + 16 * index + 8, &token.to_le_bytes());
    }
    put(&mut bytes, ORG + POINTS, &7_i32.to_le_bytes());
    put(&mut bytes, ORG + QUEUE_INDEX, &2_i32.to_le_bytes());
    put(&mut bytes, ORG + SIZE, &19_i32.to_le_bytes());
    put(&mut bytes, ORG + FUNDS, &123456_i64.to_le_bytes());
    put(&mut bytes, ORG + TASK_CAPACITY, &9_i32.to_le_bytes());
    let row = run(&bytes).unwrap();
    assert_eq!(row.queued, [77, 42, 77, u32::MAX]);
    assert_eq!(
        (
            row.state.points,
            row.state.queue_index,
            row.state.size,
            row.state.funds_raw,
            row.state.task_capacity
        ),
        (7, 2, 19, 123456, 9)
    );
    assert_eq!(row.variables.seed, Some(1));
}
#[test]
fn optional_source_and_cooldown_ignore_inactive_bytes() {
    let mut bytes = fixture();
    put(&mut bytes, ORG + SOURCE_LOCATION, &[0xff; 8]);
    put(
        &mut bytes,
        ORG + COOLDOWN + cgame_date::DATE_BASE,
        &(-912_i32).to_le_bytes(),
    );
    assert_eq!(run(&bytes).unwrap().state.source_location, None);
    assert_eq!(run(&bytes).unwrap().state.cooldown_hour, None);
    bytes[ORG + SOURCE_LOCATION_ACTIVE] = 1;
    bytes[ORG + COOLDOWN_ACTIVE] = 1;
    let row = run(&bytes).unwrap();
    assert_eq!(row.state.source_location, Some([u32::MAX; 2]));
    assert_eq!(row.state.cooldown_hour, Some(-912));
}
#[test]
fn queue_uses_the_live_count_and_refuses_a_fault_in_its_last_entry() {
    let mut bytes = fixture();
    bytes.resize(QUEUE + 5000 * 16, 0);
    array(&mut bytes, ORG + QUEUED, QUEUE, 5000);
    put(&mut bytes, QUEUE + 4999 * 16 + 8, &991_u32.to_le_bytes());
    let row = run(&bytes).unwrap();
    assert_eq!(row.queued.len(), 5000);
    assert_eq!(row.queued[4999], 991);
    bytes.truncate(QUEUE + 4999 * 16 + 11);
    assert_eq!(
        run(&bytes).unwrap_err(),
        "organisation detail field unreadable"
    );
}
#[test]
fn empty_task_slots_survive_and_bad_task_identity_refuses_details() {
    let mut bytes = fixture();
    array(&mut bytes, ORG + super::super::PRODUCTION_LINES, QUEUE, 2);
    for offset in [0, 24] {
        put(
            &mut bytes,
            QUEUE + offset,
            &(TASK_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
    }
    assert_eq!(run(&bytes).unwrap().tasks, vec![Task::default(); 2]);
    bytes[QUEUE] ^= 1;
    assert_eq!(
        run(&bytes).unwrap_err(),
        "organisation task vtable mismatch"
    );
}

#[test]
fn daily_scalars_observe_direct_assignments_while_certified_rosters_and_stores_carry() {
    let mut bytes = fixture();
    let mut row = run(&bytes).unwrap();
    row.queued = vec![9, 3, 9];
    row.state.policy = 111;
    put(&mut bytes, ORG + POLICY, &771_u32.to_le_bytes());
    for (offset, value) in [
        (RESEARCH_BONUS, 123_i64),
        (RESEARCH_ASSIGN_COST, 456),
        (PRODUCTION_ASSIGN_COST, 789),
        (FUNDS_GAIN_FACTOR, 654),
        (SIZE_COST_FACTOR, 321),
    ] {
        put(&mut bytes, ORG + offset, &value.to_le_bytes());
    }
    put(
        &mut bytes,
        ORG + DESIGN_TEAM_CHANGE_COST,
        &17_i32.to_le_bytes(),
    );
    put(&mut bytes, ORG + TASK_CAPACITY, &11_i32.to_le_bytes());
    put(&mut bytes, ORG + UPGRADES, &[0]);
    put(&mut bytes, ORG + COOLDOWN_ACTIVE, &[1]);
    put(
        &mut bytes,
        ORG + COOLDOWN + cgame_date::DATE_BASE,
        &654321_i32.to_le_bytes(),
    );
    put(&mut bytes, ORG + SOURCE_LOCATION_ACTIVE, &[0]);
    put(&mut bytes, ORG + UNLOCKED, &0_u64.to_le_bytes());
    put(&mut bytes, ORG + VARIABLES, &0_u64.to_le_bytes());
    put(&mut bytes, ORG + FLAGS, &0_u64.to_le_bytes());
    put(&mut bytes, ORG + FUNDS, &998877_i64.to_le_bytes());
    put(&mut bytes, ORG + SIZE, &4_i32.to_le_bytes());
    put(&mut bytes, ORG + POINTS, &2_i32.to_le_bytes());
    put(&mut bytes, ORG + QUEUE_INDEX, &1_i32.to_le_bytes());
    capture_selected(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        ORG,
        (17_u64 << 32) | 79,
        5,
        &mut row,
        &mut Scratch::default(),
        Freshness {
            rosters: false,
            variables: false,
            flags: false,
        },
    )
    .unwrap();
    assert_eq!(row.queued, [9, 3, 9]);
    assert_eq!(row.state.policy, 771);
    assert_eq!(row.state.upgrades, 0);
    assert_eq!(row.state.cooldown_hour, Some(654321));
    assert_eq!(row.state.source_location, None);
    assert_eq!(
        (
            row.state.research_bonus_raw,
            row.state.research_assign_cost_raw,
            row.state.production_assign_cost_raw,
            row.state.funds_gain_factor_raw,
            row.state.size_cost_factor_raw
        ),
        (123, 456, 789, 654, 321)
    );
    assert_eq!(
        (row.state.design_team_change_cost, row.state.task_capacity),
        (17, 11)
    );
    assert_eq!(
        (
            row.state.funds_raw,
            row.state.size,
            row.state.points,
            row.state.queue_index
        ),
        (998877, 4, 2, 1)
    );
}

#[test]
fn research_and_production_slots_keep_referent_identity_and_reject_reused_line_storage() {
    let mut bytes = fixture();
    array(&mut bytes, ORG + super::super::PRODUCTION_LINES, QUEUE, 2);
    let technology = 0x4000;
    let status = 0x4300;
    let country = 0x4700;
    let template = 0x4900;
    let line = 0x5000;
    for index in 0..2 {
        put(
            &mut bytes,
            QUEUE + 24 * index,
            &(TASK_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
    }
    put(&mut bytes, QUEUE + 8, &(technology as u64).to_le_bytes());
    put(
        &mut bytes,
        technology,
        &(TECHNOLOGY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        technology + TECHNOLOGY_HANDLE,
        &91_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        technology + TECHNOLOGY_STATUS,
        &(status as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        status + TECHNOLOGY_STATUS_COUNTRY,
        &(country as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        country + engine_native::ccountry::ID,
        &31_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        technology + TECHNOLOGY_TEMPLATE,
        &(template as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        template + TECHNOLOGY_TEMPLATE_INDEX,
        &(-3_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        template + TECHNOLOGY_TEMPLATE_TOKEN,
        &101_u32.to_le_bytes(),
    );
    put(&mut bytes, QUEUE + 24 + 16, &(line as u64).to_le_bytes());
    put(
        &mut bytes,
        line,
        &(engine_native_parts::cmilitary_production_line::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    let reference = (17_u64 << 32) | 51;
    put(
        &mut bytes,
        line + engine_native_parts::cproduction_line::REFERENCE_TYPE,
        &reference.to_le_bytes(),
    );
    let row = run(&bytes).unwrap();
    assert_eq!(
        row.tasks[0].technology,
        Some(Technology {
            country: 31,
            handle: 91,
            index: -3,
            token: 101
        })
    );
    assert_eq!(row.tasks[1].line, Some(reference));
    bytes[line] ^= 1;
    assert_eq!(
        run(&bytes).unwrap_err(),
        "organisation task line identity mismatch"
    );
}

#[test]
fn country_policies_keep_cached_names_context_and_selection_score() {
    let mut bytes = fixture();
    let status = 0x4000;
    let policy = 0x4500;
    array(
        &mut bytes,
        status + engine_native::cproduction_status::POLICIES,
        QUEUE,
        1,
    );
    put(&mut bytes, QUEUE, &(policy as u64).to_le_bytes());
    put(
        &mut bytes,
        policy,
        &(engine_native::cproduction_status::POLICY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        policy + engine_native::cproduction_status::POLICY_TOKEN,
        &19_u32.to_le_bytes(),
    );
    put(&mut bytes, policy + POLICY_COST, &(-7_i32).to_le_bytes());
    put(&mut bytes, policy + POLICY_DAYS, &31_i32.to_le_bytes());
    put(&mut bytes, policy + POLICY_COUNTRY, &u32::MAX.to_le_bytes());
    put(
        &mut bytes,
        policy + POLICY_CACHED_SCORE,
        &i64::MIN.to_le_bytes(),
    );
    put(&mut bytes, policy + POLICY_NAME, &[0xff, 0, b'k']);
    put(&mut bytes, policy + POLICY_NAME + 16, &3_u64.to_le_bytes());
    put(&mut bytes, policy + POLICY_NAME + 24, &15_u64.to_le_bytes());
    let mut rows = Vec::new();
    capture_policies(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        status,
        &mut rows,
    )
    .unwrap();
    assert_eq!(
        rows,
        vec![Policy {
            token: 19,
            cost: -7,
            days: 31,
            country: u32::MAX,
            name: vec![0xff, 0, b'k'],
            cached_score_raw: i64::MIN
        }]
    );
    put(&mut bytes, policy + POLICY_COST, &23_i32.to_le_bytes());
    put(&mut bytes, policy + POLICY_DAYS, &71_i32.to_le_bytes());
    put(&mut bytes, policy + POLICY_NAME + 24, &0_u64.to_le_bytes());
    capture_policies_selected(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        status,
        &mut rows,
        false,
    )
    .unwrap();
    assert_eq!((rows[0].cost, rows[0].days), (23, 71));
    assert_eq!(rows[0].name, [0xff, 0, b'k']);
    assert_eq!(
        (rows[0].country, rows[0].cached_score_raw),
        (u32::MAX, i64::MIN)
    );
    bytes[policy] ^= 1;
    assert_eq!(
        capture_policies(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            status,
            &mut rows
        )
        .unwrap_err(),
        "organisation policy vtable mismatch"
    );
}

#[test]
fn policy_cost_modifier_is_the_country_accumulators_effective_entry() {
    use engine_native_parts::modifier_definition as definitions;
    use crate::modifier_keys::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST as KEY;
    use std::collections::BTreeMap;
    const COUNTRY: usize = 0x1_0000;
    const VALUES: usize = 0x2_0000;
    const DEFINITIONS: usize = 0x3_0000;
    #[derive(Default)]
    struct Sparse(BTreeMap<usize, u8>);
    impl Memory for Sparse {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            for (index, byte) in output.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl Sparse {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                self.0.insert(at + index, *byte);
            }
        }
        fn entries(&mut self, entries: &[(u32, i64)]) {
            let header = COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
            self.put(header, &(VALUES as u64).to_le_bytes());
            self.put(header + 12, &(entries.len() as u32).to_le_bytes());
            for (index, (key, value)) in entries.iter().enumerate() {
                self.put(VALUES + index * 16, &key.to_le_bytes());
                self.put(VALUES + index * 16 + 8, &value.to_le_bytes());
            }
        }
        fn flags(&mut self, flags: u32) {
            self.put(
                definitions::TABLE_RVA.0,
                &(DEFINITIONS as u64).to_le_bytes(),
            );
            let row = DEFINITIONS + KEY as usize * definitions::STRIDE;
            self.put(row + definitions::FLAGS, &flags.to_le_bytes());
            self.put(row + definitions::FEATURE_SELECTOR, &0_u32.to_le_bytes());
        }
    }
    let mut memory = Sparse::default();
    memory.entries(&[(KEY - 1, 11), (KEY, -75_000), (KEY + 1, 13)]);
    memory.flags(0);
    assert_eq!(policy_cost_modifier(&memory, 0, COUNTRY), Ok(-75_000));
    memory.flags(definitions::CLAMP_MAXIMUM);
    assert_eq!(policy_cost_modifier(&memory, 0, COUNTRY), Ok(0));
    memory.entries(&[(KEY - 1, 11), (KEY + 1, 13)]);
    assert_eq!(policy_cost_modifier(&memory, 0, COUNTRY), Ok(0));
    assert!(policy_cost_modifier(&memory, 0, COUNTRY + 0x100).is_err());
}
