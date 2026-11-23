use super::{
    Capacities, Captured, ConstructionRestCopyContext, ConstructionRestCopyReport,
    ConstructionRestJob, N,
};
use engine_intelligence::ccountry_collaboration_status;
use engine_native_parts::{
    carmy_manpower_values,
    cbuilding,
    cproduction_line,
    cstate_template,
};
use engine_native::{
    cprovince::buildings as cbuilding_status,
    ccountry,
    cgame_state,
    cproduction_status,
    cstate,
};
use engine_native_memory::msvc_string;
use engine_occupation::{
    ccountry_occupation_data, ccountry_occupation_status, coccupation_law, cresistance,
    cstate_garrison_data,
};
use engine_production::{
    cbuilding_production_line, cgeneral_production_line, crailway_production_line,
};

pub const BASE: usize = 0x1000;
pub(super) const COUNTRY: usize = 0x3000;
const STATE: usize = 0x4000;
const TEMPLATE: usize = 0x5000;
const RESDATA: usize = 0x5200;
const PROD: usize = 0x6000;
const QDATA: usize = 0x6100;
const BLINE: usize = 0x9000;
const RLINE: usize = 0xA000;
const BTARGET: usize = 0xB000;
const STATUS_OBJ: usize = STATE + cstate::BUILDING_STATUS;
const RDATA: usize = 0xC000;
const OCC: usize = 0x7000;
const OCC_TABLE: usize = 0x7100;
const OCC_ROWS: usize = 0x7200;
pub(super) const OCC_DATA: usize = 0x7300;
const LAW_DATA: usize = 0x7500;
const GAR_DATA: usize = 0x7700;
pub(super) const GAR_OBJ: usize = 0x7800;
pub(super) const LAW_OBJ: usize = 0x7900;
const RESERVE_DATA: usize = 0x7B00;
const ASSIGN_DATA: usize = 0x7D00;
const COLLAB: usize = 0x8000;
const CREL_DATA: usize = 0x8100;
const CREL: usize = 0x8200;
const CINC_DATA: usize = 0x8300;

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
    array(&mut bytes, BASE + cgame_state::STATES, 0x2400, 2);
    array(&mut bytes, BASE + cgame_state::PROVINCES, 0x2500, 2);
    for slot in [1, 2] {
        put(
            &mut bytes,
            0x2000 + slot * 8,
            &(COUNTRY as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x2100 + slot * 4, &1_u32.to_le_bytes());
    }
    put(&mut bytes, 0x2400 + 8, &(STATE as u64).to_le_bytes());
    put(
        &mut bytes,
        COUNTRY,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY + ccountry::PRODUCTION_STATUS,
        &(PROD as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::OCCUPATION_STATUS,
        &(OCC as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::COLLABORATION_STATUS,
        &(COLLAB as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATE,
        &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, STATE + cstate::ID, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        STATE + cstate::TEMPLATE,
        &(TEMPLATE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE,
        &(cstate_template::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE + cstate_template::ID,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE + cstate_template::RESOURCES,
        &(RESDATA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE + cstate_template::RESOURCE_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE + cstate_template::RESOURCE_COUNT,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, RESDATA, &1234567_i64.to_le_bytes());
    put(&mut bytes, RESDATA + 8, &55_u32.to_le_bytes());
    let res = STATE + cstate::RESISTANCE;
    put(
        &mut bytes,
        res,
        &(cresistance::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        res + cresistance::STATE,
        &(STATE as u64).to_le_bytes(),
    );
    put(&mut bytes, res + cresistance::LEVEL, &100_i64.to_le_bytes());
    put(&mut bytes, res + cresistance::SPEED, &200_i64.to_le_bytes());
    put(
        &mut bytes,
        res + cresistance::COMPLIANCE,
        &300_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        res + cresistance::COMPLIANCE_SPEED,
        &400_i64.to_le_bytes(),
    );
    put(&mut bytes, res + cresistance::COUNTRY, &1_u32.to_le_bytes());
    put(&mut bytes, res + cresistance::ACTIVE, &[1]);
    put(
        &mut bytes,
        PROD,
        &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        PROD + cproduction_status::COUNTRY,
        &(COUNTRY as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        PROD + cproduction_status::CONSTRUCTION_LINES,
        QDATA,
        2,
    );
    put(&mut bytes, QDATA, &(BLINE as u64).to_le_bytes());
    put(&mut bytes, QDATA + 8, &(RLINE as u64).to_le_bytes());
    for (pointer, vtable, id) in [
        (
            BLINE,
            cbuilding_production_line::VTABLE_RVA,
            0x0000_0001_ffff_ffff_u64,
        ),
        (
            RLINE,
            crailway_production_line::VTABLE_RVA,
            0xffff_ffff_0000_0001_u64,
        ),
    ] {
        put(&mut bytes, pointer, &(vtable.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            pointer + cproduction_line::REFERENCE_TYPE,
            &id.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::PRODUCTION_STATUS,
            &(PROD as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::ACTIVE_FACTORIES,
            &3_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::COST,
            &10_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::SPEED,
            &20_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::PRODUCED,
            &30_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::PRIORITY,
            &1_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cproduction_line::AMOUNT,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cgeneral_production_line::CREATED_DATE,
            &100_i32.to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        BLINE + cbuilding_production_line::BUILDING,
        &(BTARGET as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        BLINE + cbuilding_production_line::REPAIR_LEVELS_REMAINING,
        &0_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BTARGET,
        &(cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        BTARGET + cbuilding::TYPE_ID,
        &7_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BTARGET + cbuilding::BUILDING_STATUS,
        &(STATUS_OBJ as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_OBJ,
        &(cbuilding_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_OBJ + cbuilding_status::FLAGS,
        &0_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_OBJ + cbuilding_status::FLAGS + 4,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_OBJ + cbuilding_status::FLAGS + 8,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_OBJ + cbuilding_status::ID,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_OBJ + cbuilding_status::COUNTRY,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        RLINE + crailway_production_line::BASE_LEVEL,
        &1_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        RLINE + crailway_production_line::TARGET_LEVEL,
        &2_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        RLINE + crailway_production_line::COMPLETION_FLAG,
        &[0],
    );
    array(
        &mut bytes,
        RLINE + crailway_production_line::ROUTE,
        RDATA,
        2,
    );
    put(&mut bytes, RDATA, &9_u32.to_le_bytes());
    put(&mut bytes, RDATA + 4, &10_u32.to_le_bytes());
    put(
        &mut bytes,
        RLINE + crailway_production_line::CURRENT_SEGMENT,
        &0_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        RLINE + crailway_production_line::REMAINING_COST,
        &50_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC,
        &(ccountry_occupation_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::SECONDARY_BASE,
        &(ccountry_occupation_status::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::GARRISON_TEMPLATE,
        &0_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::DEFAULT_LAW,
        &(LAW_OBJ as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LAW_OBJ,
        &(coccupation_law::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LAW_OBJ + coccupation_law::NAME,
        b"civilian\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    );
    put(
        &mut bytes,
        LAW_OBJ + coccupation_law::NAME + msvc_string::LENGTH,
        &8_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        LAW_OBJ + coccupation_law::NAME + msvc_string::CAPACITY,
        &15_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::ASSIGNED_MANPOWER,
        &(carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        OCC + ccountry_occupation_status::ASSIGNED_MANPOWER + carmy_manpower_values::DATA,
        RESERVE_DATA,
        1,
    );
    put(&mut bytes, RESERVE_DATA, &1_u32.to_le_bytes());
    put(&mut bytes, RESERVE_DATA + 4, &(-17_i32).to_le_bytes());
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::OCCUPATION_DATA_TABLE,
        &(OCC_TABLE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::OCCUPATION_DATA_TABLE + 8,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::OCCUPATION_DATA_TABLE + 12,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, OCC_TABLE + 4, &[1]);
    put(&mut bytes, OCC_TABLE + 8, &1_u32.to_le_bytes());
    put(&mut bytes, OCC_TABLE + 16, &(OCC_DATA as u64).to_le_bytes());
    array(
        &mut bytes,
        OCC + ccountry_occupation_status::OCCUPATION_DATA_ROWS,
        OCC_ROWS,
        1,
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::OCCUPATION_DATA_ROWS + 8,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC + ccountry_occupation_status::OCCUPATION_DATA_ROWS + 12,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, OCC_ROWS, &(OCC_DATA as u64).to_le_bytes());
    put(
        &mut bytes,
        OCC_DATA,
        &(ccountry_occupation_data::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC_DATA + ccountry_occupation_data::PARENT_STATUS,
        &(OCC as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC_DATA + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER,
        &(COUNTRY as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        OCC_DATA + ccountry_occupation_data::OCCUPIED_COUNTRY,
        &1_u32.to_le_bytes(),
    );
    array(
        &mut bytes,
        OCC_DATA + ccountry_occupation_data::LAW_TABLE,
        LAW_DATA,
        1,
    );
    put(&mut bytes, LAW_DATA + 4, &[1]);
    put(&mut bytes, LAW_DATA + 8, &0_u32.to_le_bytes());
    put(&mut bytes, LAW_DATA + 16, &(LAW_OBJ as u64).to_le_bytes());
    array(
        &mut bytes,
        OCC_DATA + ccountry_occupation_data::GARRISON_TABLE,
        GAR_DATA,
        1,
    );
    put(&mut bytes, GAR_DATA + 4, &[1]);
    put(&mut bytes, GAR_DATA + 8, &1_u32.to_le_bytes());
    put(&mut bytes, GAR_DATA + 16, &(GAR_OBJ as u64).to_le_bytes());
    put(
        &mut bytes,
        GAR_OBJ,
        &(cstate_garrison_data::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::STATE,
        &(STATE as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::OCCUPATION_DATA,
        &(OCC_DATA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::STRENGTH_RATIO,
        &60_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::GARRISON_REQUIRED,
        &70_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::REQUIRED_MANPOWER,
        &80_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::ASSIGNED_MANPOWER,
        &(carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        GAR_OBJ + cstate_garrison_data::ASSIGNED_MANPOWER + carmy_manpower_values::DATA,
        ASSIGN_DATA,
        1,
    );
    put(&mut bytes, ASSIGN_DATA, &1_u32.to_le_bytes());
    put(&mut bytes, ASSIGN_DATA + 4, &90_i32.to_le_bytes());
    put(
        &mut bytes,
        COLLAB,
        &(ccountry_collaboration_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COLLAB + ccountry_collaboration_status::OWNER,
        &(COUNTRY as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        COLLAB + ccountry_collaboration_status::RELATIONS,
        CREL_DATA,
        0,
    );
    array(
        &mut bytes,
        COLLAB + ccountry_collaboration_status::INCOMING_COUNTRIES,
        CINC_DATA,
        0,
    );
    let _ = CREL;
    bytes
}

pub fn capacities() -> Capacities {
    Capacities {
        rows: [8; N],
        routes: 8,
        law_bytes: 64,
        occupied: 8,
        group_laws: 8,
        equipment: 0,
    }
}

pub fn direct(memory: &impl engine_native_memory::Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let mut buildings = captured.take_buildings();
    let ticks = buildings.capture(memory, 0, BASE, 0, true);
    captured.restore_buildings(buildings, Ok(ticks));
    let _ = captured.capture(memory, 0, BASE, 0, true);
    captured.finish_buildings(0, true);
    captured.rest_raw_mode = false;
    captured
}

pub fn job() -> ConstructionRestJob {
    let mut job = ConstructionRestJob::new();
    job.log = crate::raw::RawLog::new(1 << 17, 8 << 20);
    job.census.reserve_exact(16);
    job.update_capacity_charge();
    job
}

pub fn split(
    memory: &impl engine_native_memory::Memory,
    capacities: &Capacities,
) -> (Captured, ConstructionRestCopyReport, ConstructionRestJob) {
    let mut captured = Captured::with_capacities(capacities);
    let ctx = ConstructionRestCopyContext {
        base: 0,
        game_state: BASE,
    };
    let mut first = job();
    let report = crate::construction::copy_construction_rest_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    captured.rest_raw_mode = report.raw_ok;
    let mut buildings = captured.take_buildings();
    let ticks = buildings.capture(memory, 0, BASE, 0, true);
    captured.restore_buildings(buildings, Ok(ticks));
    let _ = captured.capture(memory, 0, BASE, 0, true);
    captured.finish_buildings(0, true);
    let mut ticks = [0u64; 10];
    captured.decode_construction_rest_payload(0, BASE, &first, &mut ticks);
    (captured, report, first)
}

pub fn assert_matches_direct(split: &Captured, direct: &Captured) {
    assert_eq!(
        split.base_resources.outcome, direct.base_resources.outcome,
        "resources outcome"
    );
    assert_eq!(
        split.building_construction.outcome, direct.building_construction.outcome,
        "building outcome"
    );
    assert_eq!(
        split.railway_construction.outcome, direct.railway_construction.outcome,
        "railway outcome"
    );
    assert_eq!(
        split.settings.outcome, direct.settings.outcome,
        "settings outcome"
    );
    assert_eq!(split.laws.outcome, direct.laws.outcome, "laws outcome");
    assert_eq!(
        split.group_laws.outcome, direct.group_laws.outcome,
        "group laws outcome"
    );
    assert_eq!(split.fallback_law, direct.fallback_law, "fallback law");
    let len = split.group_laws.outcome.len();
    assert_eq!(
        &split.group_laws.rows[..len],
        &direct.group_laws.rows[..len],
        "group law rows"
    );
    assert_eq!(
        split.garrisons.outcome, direct.garrisons.outcome,
        "garrisons outcome"
    );
    assert_eq!(
        split.assigned_manpower.outcome, direct.assigned_manpower.outcome,
        "assigned outcome"
    );
    assert_eq!(
        split.reserve_manpower.outcome, direct.reserve_manpower.outcome,
        "reserve outcome"
    );
    assert_eq!(
        split.resistance.outcome, direct.resistance.outcome,
        "resistance outcome"
    );
    assert_eq!(
        split.collaboration.outcome, direct.collaboration.outcome,
        "collaboration outcome"
    );
    let len = split.base_resources.outcome.len();
    assert_eq!(
        &split.base_resources.rows[..len],
        &direct.base_resources.rows[..len],
        "resource rows"
    );
    let len = split.building_construction.outcome.len();
    assert_eq!(
        &split.building_construction.rows[..len],
        &direct.building_construction.rows[..len],
        "building rows"
    );
    let len = split.railway_construction.outcome.len();
    assert_eq!(
        &split.railway_construction.rows[..len],
        &direct.railway_construction.rows[..len],
        "railway rows"
    );
    assert_eq!(split.routes.len, direct.routes.len, "routes arena");
    assert_eq!(
        &split.routes.first[..split.routes.len],
        &direct.routes.first[..direct.routes.len],
        "route provinces"
    );
    assert_eq!(split.law_bytes.len, direct.law_bytes.len, "law arena");
    assert_eq!(
        &split.law_bytes.first[..split.law_bytes.len],
        &direct.law_bytes.first[..direct.law_bytes.len],
        "law bytes"
    );
    assert_eq!(split.occupied.len, direct.occupied.len, "occupied arena");
    assert_eq!(
        &split.occupied.first[..split.occupied.len],
        &direct.occupied.first[..direct.occupied.len],
        "occupied countries"
    );
    let len = split.settings.outcome.len();
    assert_eq!(
        &split.settings.rows[..len],
        &direct.settings.rows[..len],
        "settings rows"
    );
    let len = split.laws.outcome.len();
    assert_eq!(
        &split.laws.rows[..len],
        &direct.laws.rows[..len],
        "law rows"
    );
    let len = split.resistance.outcome.len();
    assert_eq!(
        &split.resistance.rows[..len],
        &direct.resistance.rows[..len],
        "resistance rows"
    );
}
