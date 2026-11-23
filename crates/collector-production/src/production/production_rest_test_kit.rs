use super::{
    Capacities, Captured, N, ProductionRestCopyContext, ProductionRestCopyReport,
    ProductionRestJob, Sources,
};
use engine_deployment::{
    cdeployment_status, cmilitary_deployment, cmilitary_deployment_conveyor,
    cmilitary_deployment_line,
};
use engine_native_parts::{carmy_manpower_values, cproduction_line};
use engine_native::{
    ccountry,
    cproduction_status::{
        equipment_license as cequipment_production_license,
        equipment_line as cequipment_production_line,
        self,
    },
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cgame_state,
    cname_group_member,
    creferenced_division_template,
};
use engine_native_memory::{msvc_string, walk::Outcome};

pub const BASE: usize = 0x1000;
const COUNTRY: usize = 0x3000;
const PROD: usize = 0x10000;
const DEPLOY: usize = 0x4000;
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
pub(super) const VARIANT: usize = 0x7700;
pub(super) const PLINE: usize = 0x11000;
const PDATA: usize = 0x11100;
const SDATA: usize = 0x11200;
const LDATA: usize = 0x11300;
const RDATA: usize = 0x11400;
const RECORD: usize = 0x11500;
const RLIST: usize = 0x11600;
const EDATA: usize = 0x11700;
pub(super) const EQUIPMENT_TYPE: usize = 0x12000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
    put(bytes, header, &(data as u64).to_le_bytes());
    put(bytes, header + 8, &count.to_le_bytes());
    put(bytes, header + 12, &count.to_le_bytes());
}

pub fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x12000];
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
        COUNTRY + ccountry::PRODUCTION_STATUS,
        &(PROD as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::DEPLOYMENT_MANAGER,
        &(DEPLOY as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        PROD,
        &(cproduction_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        PROD + cproduction_status::COUNTRY,
        &(COUNTRY as u64).to_le_bytes(),
    );
    array(&mut bytes, PROD + cproduction_status::LINES, PDATA, 1);
    put(&mut bytes, PDATA, &(PLINE as u64).to_le_bytes());
    array(&mut bytes, PROD + cproduction_status::LICENSES, LDATA, 1);
    put(&mut bytes, LDATA, &(VARIANT as u64).to_le_bytes());
    array(
        &mut bytes,
        PROD + cproduction_status::LICENSE_RECORDS,
        RDATA,
        1,
    );
    put(&mut bytes, RDATA, &(RECORD as u64).to_le_bytes());
    put(
        &mut bytes,
        PROD + cproduction_status::STOCKPILE_POOL,
        &(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(&mut bytes, PROD + cproduction_status::STOCKPILE, SDATA, 1);
    put(&mut bytes, SDATA, &(VARIANT as u64).to_le_bytes());
    put(&mut bytes, SDATA + 8, &77_i64.to_le_bytes());
    for (offset, value) in [
        (
            0,
            engine_native_parts::cmilitary_production_line::VTABLE_RVA.0 as u64,
        ),
        (cproduction_line::REFERENCE_TYPE, 0x8000_1234_0000_0038),
        (cproduction_line::PRODUCTION_STATUS, PROD as u64),
        (cproduction_line::COST, 100),
        (cproduction_line::SPEED, 200),
        (cproduction_line::PRODUCED, 300),
        (
            cequipment_production_line::EQUIPMENT_VARIANT,
            VARIANT as u64,
        ),
        (
            cequipment_production_line::INDUSTRIAL_ORGANISATION_CID,
            0x1234_5678_0000_0095,
        ),
    ] {
        put(&mut bytes, PLINE + offset, &value.to_le_bytes());
    }
    for (offset, value) in [
        (cproduction_line::ACTIVE_FACTORIES, 12_i32),
        (cproduction_line::PRIORITY, 17),
        (cproduction_line::AMOUNT, -1),
        (cequipment_production_line::REQUESTED_FACTORIES, 14),
    ] {
        put(&mut bytes, PLINE + offset, &value.to_le_bytes());
    }
    array(
        &mut bytes,
        PLINE + cequipment_production_line::FACTORY_EFFICIENCIES,
        EDATA,
        20,
    );
    for index in 0..20 {
        let value = (index as i64 + 40) << 15;
        put(&mut bytes, EDATA + index * 8, &value.to_le_bytes());
    }
    put(
        &mut bytes,
        VARIANT + cequipment_variant::DEFINITION,
        &(EQUIPMENT_TYPE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        EQUIPMENT_TYPE,
        &(engine_native_parts::cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        EQUIPMENT_TYPE + engine_native_parts::cequipment_type::ARCHETYPE,
        &(EQUIPMENT_TYPE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        EQUIPMENT_TYPE + engine_native_parts::cequipment_type::MAX_MIL_FACTORIES,
        &[150, 0, 0, 0, 1],
    );
    put(
        &mut bytes,
        VARIANT,
        &(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        VARIANT + cequipment_variant::TYPE,
        &0xfedc_ba98_0000_0046_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        RECORD,
        &(cequipment_production_license::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        RECORD + cequipment_production_license::OWNER,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        RECORD + cequipment_production_license::UNSERIALIZED_OPERAND,
        &100_000_i64.to_le_bytes(),
    );
    for (offset, value) in [
        (cequipment_production_license::LENDED_CIC, 3_i32),
        (cequipment_production_license::REQUIRED_CIC, 4_i32),
        (cequipment_production_license::GIVER, 5),
        (cequipment_production_license::START_DATE, 6),
    ] {
        put(&mut bytes, RECORD + offset, &value.to_le_bytes());
    }
    put(
        &mut bytes,
        RECORD + cequipment_production_license::PARENT,
        &(VARIANT as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        RECORD + cequipment_production_license::EQUIPMENT,
        RLIST,
        1,
    );
    put(&mut bytes, RLIST, &(VARIANT as u64).to_le_bytes());
    put(
        &mut bytes,
        DEPLOY,
        &(cdeployment_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOY + cdeployment_status::OWNER,
        &(COUNTRY as u64).to_le_bytes(),
    );
    array(&mut bytes, DEPLOY + cdeployment_status::CONVEYORS, QUEUE, 1);
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
        CONVEYOR + cmilitary_deployment_conveyor::NAME + msvc_string::LENGTH,
        &6_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        CONVEYOR + cmilitary_deployment_conveyor::NAME + msvc_string::CAPACITY,
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
        &9_u32.to_le_bytes(),
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
        &11_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        NAME_OBJECT + cname_group_member::OVERRIDE_NAME + msvc_string::LENGTH,
        &0_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        NAME_OBJECT + cname_group_member::OVERRIDE_NAME + msvc_string::CAPACITY,
        &msvc_string::INLINE_CAPACITY.to_le_bytes(),
    );
    put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_A, &[1]);
    put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_B, &[0]);
    put(
        &mut bytes,
        DEPLOYMENT,
        &(cmilitary_deployment::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::SECONDARY_VTABLE,
        &(cmilitary_deployment::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL,
        &(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        EQUIPMENT,
        1,
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL + cequipment_variant_pool::ALLOW_ZERO,
        &[1],
    );
    array(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL + cequipment_variant_pool::ALLOCATIONS,
        ALLOCATIONS,
        0,
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::DISTRIBUTABLE_PRIORITY,
        &2_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::HALTING_REASON,
        &3_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::HALTING_REASON_PRESENT,
        &[1],
    );
    put(&mut bytes, EQUIPMENT, &(VARIANT as u64).to_le_bytes());
    put(&mut bytes, EQUIPMENT + 8, &55_i64.to_le_bytes());
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::TRAINING,
        &0_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::MAX_TRAINING,
        &66_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::MAX_MANPOWER,
        &77_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEPLOYMENT + cmilitary_deployment::OWNING_LINE,
        &(LINE as u64).to_le_bytes(),
    );
    for (offset, container, count) in [
        (cmilitary_deployment::MANPOWER_HAVE, HAVE, 1_u32),
        (cmilitary_deployment::MANPOWER_NEED, NEED, 0),
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
    put(&mut bytes, HAVE, &8_u32.to_le_bytes());
    put(&mut bytes, HAVE + 4, &(-5_i32).to_le_bytes());
    bytes
}

pub const MANY: usize = 3;
const MORE_LINES: usize = 0x11800;
const LINE_SIZE: usize = 0x100;

pub fn line_produced(line: usize) -> i64 {
    300 + 11 * line as i64
}

pub fn stockpile_amount(entry: usize) -> i64 {
    77 + 5 * entry as i64
}

pub fn many_rows_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    let template = bytes[PLINE - BASE..PLINE - BASE + LINE_SIZE].to_vec();
    array(&mut bytes, PROD + cproduction_status::LINES, PDATA, MANY as u32);
    array(&mut bytes, PROD + cproduction_status::STOCKPILE, SDATA, MANY as u32);
    for index in 0..MANY {
        let line = if index == 0 { PLINE } else { MORE_LINES + (index - 1) * LINE_SIZE };
        put(&mut bytes, line, &template);
        put(&mut bytes, line + cproduction_line::REFERENCE_TYPE, &(0x8000_1234_0000_0038_u64 + ((index as u64) << 32)).to_le_bytes());
        put(&mut bytes, line + cproduction_line::PRODUCED, &(line_produced(index) as u64).to_le_bytes());
        put(&mut bytes, PDATA + 8 * index, &(line as u64).to_le_bytes());
        put(&mut bytes, SDATA + 16 * index, &(VARIANT as u64).to_le_bytes());
        put(&mut bytes, SDATA + 16 * index + 8, &stockpile_amount(index).to_le_bytes());
    }
    bytes
}

pub fn capacities() -> Capacities {
    Capacities {
        rows: [8; N],
        definition_bytes: 64,
        conveyor_name_bytes: 64,
        line_custom_name_bytes: 64,
        license_handles: 64,
    }
}

pub fn direct(memory: &impl engine_native_memory::Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let divisions = Sources {
        rows: &[],
        outcome: Outcome::Rows { len: 0 },
    };
    let _ = captured.acquire(memory, 0, BASE, divisions, 0, true);
    captured.rest_raw_mode = false;
    captured
}

pub fn job() -> ProductionRestJob {
    let mut job = ProductionRestJob::new();
    job.log = crate::raw::RawLog::new(8_192, 1 << 20);
    job.log.initialize_read_plan(8_192, 8_192, 1 << 20);
    job.census.reserve_exact(16);
    job.update_capacity_charge();
    job
}

pub fn split(
    memory: &impl engine_native_memory::Memory,
    capacities: &Capacities,
) -> (Captured, ProductionRestCopyReport, ProductionRestJob) {
    let mut captured = Captured::with_capacities(capacities);
    let ctx = ProductionRestCopyContext {
        base: 0,
        game_state: BASE,
    };
    let mut first = job();
    let report = crate::production::copy_production_rest_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    captured.rest_raw_mode = report.raw_ok;
    let divisions = Sources {
        rows: &[],
        outcome: Outcome::Rows { len: 0 },
    };
    let _ = captured.acquire(memory, 0, BASE, divisions, 0, true);
    let mut ticks = [0u64; 8];
    captured.decode_production_rest_payload(0, BASE, &first, &mut ticks);
    (captured, report, first)
}

pub fn assert_matches_direct(split: &Captured, direct: &Captured) {
    assert_eq!(split.lines.outcome, direct.lines.outcome, "lines outcome");
    assert_eq!(
        split.stockpiles.outcome, direct.stockpiles.outcome,
        "stockpiles outcome"
    );
    assert_eq!(
        split.licenses.outcome, direct.licenses.outcome,
        "licences outcome"
    );
    assert_eq!(
        split.conveyors.outcome, direct.conveyors.outcome,
        "conveyors outcome"
    );
    assert_eq!(
        split.deployment_lines.outcome, direct.deployment_lines.outcome,
        "lines outcome"
    );
    assert_eq!(
        split.deployment_manpower.outcome, direct.deployment_manpower.outcome,
        "manpower outcome"
    );
    assert_eq!(
        split.deployment_equipment.outcome, direct.deployment_equipment.outcome,
        "equipment outcome"
    );
    assert_eq!(
        split.license_records.outcome, direct.license_records.outcome,
        "records outcome"
    );
    let len = split.lines.outcome.len();
    assert_eq!(
        &split.lines.rows[..len],
        &direct.lines.rows[..len],
        "line rows"
    );
    let len = split.stockpiles.outcome.len();
    assert_eq!(
        &split.stockpiles.rows[..len],
        &direct.stockpiles.rows[..len],
        "stockpile rows"
    );
    let len = split.conveyors.outcome.len();
    assert_eq!(
        &split.conveyors.rows[..len],
        &direct.conveyors.rows[..len],
        "conveyor rows"
    );
    let len = split.license_records.outcome.len();
    assert_eq!(
        &split.license_records.rows[..len],
        &direct.license_records.rows[..len],
        "record rows"
    );
    assert_eq!(
        &split.conveyor_name_bytes.first[..split.conveyor_name_bytes.len],
        &direct.conveyor_name_bytes.first[..direct.conveyor_name_bytes.len],
        "conveyor names"
    );
    assert_eq!(
        &split.license_handles.first[..split.license_handles.len],
        &direct.license_handles.first[..direct.license_handles.len],
        "licence handles"
    );
}
