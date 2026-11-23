use super::{
    Capacities, Captured, RadarCopyContext, RadarCopyReport, RadarJob, copy_radar_refresh,
};
use engine_native_memory::{Memory, memory::SliceMemory, walk::Outcome};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
const LTAB: usize = 0x12000;
pub const C0: usize = 0x20000;
pub const GROUP: usize = 0x50000;
const GVEC: usize = 0x40000;
const COVERED: usize = 0x70000;
const STATE_A: usize = 0x60000;

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put32(bytes, at + 8, count);
    put32(bytes, at + 12, count);
}

pub fn fixture() -> Vec<u8> {
    use engine_map_air::radar_group;
    use engine_native::{ccountry, cgame_state, cstate};
    use engine_native_parts::cradars_pool;
    let mut bytes = vec![0; 0x80000];
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 2);
    put64(&mut bytes, CTAB, 0);
    put64(&mut bytes, CTAB + 8, C0 as u64);
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 3);
    put32(&mut bytes, LTAB, 0);
    put32(&mut bytes, LTAB + 4, 1);
    put32(&mut bytes, LTAB + 8, 0);
    put64(&mut bytes, C0, ccountry::VTABLE_RVA.0 as u64);
    put32(&mut bytes, C0 + ccountry::ID, 1);
    put64(
        &mut bytes,
        C0 + ccountry::RADARS_POOL,
        cradars_pool::VTABLE_RVA.0 as u64,
    );
    put32(&mut bytes, C0 + ccountry::RADARS_POOL_COUNTRY, 1);
    header(&mut bytes, C0 + ccountry::RADAR_GROUPS, GVEC, 1);
    put64(&mut bytes, GVEC, GROUP as u64);
    put64(
        &mut bytes,
        GROUP + radar_group::PARENT,
        (C0 + ccountry::RADARS_POOL) as u64,
    );
    put64(&mut bytes, GROUP + radar_group::STATE, STATE_A as u64);
    header(&mut bytes, GROUP + radar_group::PROVINCES, COVERED, 2);
    put32(&mut bytes, COVERED, 11);
    put32(&mut bytes, COVERED + 4, 12);
    put64(&mut bytes, STATE_A, cstate::VTABLE_RVA.0 as u64);
    put32(&mut bytes, STATE_A + cstate::ID, 77);
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub struct Split {
    pub outcome: Outcome,
    pub rows: Vec<engine_map_air::radar_group::Row>,
    pub provinces: Vec<u32>,
}

fn collect_typed(memory: &impl Memory, rows_cap: usize, provinces_cap: usize) -> Split {
    use engine_map_air::radar_group;
    let capacities = Capacities {
        rows: [rows_cap, 4, 4, 4, 4, 4, 4],
        radar_provinces: provinces_cap,
        area_provinces: 8,
        wings: 8,
    };
    let mut captured = Captured::with_capacities(&capacities);
    let walked = radar_group::walk(
        memory,
        BASE,
        GAME_STATE,
        &mut captured.radar.rows,
        &mut captured.radar_provinces.first,
    );
    super::arena_first(
        &mut captured.radar,
        &mut captured.radar_provinces,
        walked.outcome,
        walked.provinces_len,
        walked.required_provinces,
    );
    Split {
        outcome: captured.radar.outcome,
        rows: captured.radar.rows.clone(),
        provinces: captured.radar_provinces.first.clone(),
    }
}

fn collect_split(
    memory: &impl Memory,
    rows_cap: usize,
    provinces_cap: usize,
) -> (Split, RadarCopyReport, RadarJob) {
    let ctx = RadarCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
        row_limit: rows_cap,
        province_limit: provinces_cap,
    };
    let capacities = Capacities {
        rows: [rows_cap, 4, 4, 4, 4, 4, 4],
        radar_provinces: provinces_cap,
        area_provinces: 8,
        wings: 8,
    };
    let mut captured = Captured::with_capacities(&capacities);
    let mut first = RadarJob::with_storage(rows_cap, provinces_cap);
    let report = copy_radar_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut ticks = [0u64; 1];
    captured.decode_radar_payload(&first, &mut ticks);
    (
        Split {
            outcome: captured.radar.outcome,
            rows: captured.radar.rows.clone(),
            provinces: captured.radar_provinces.first.clone(),
        },
        report,
        first,
    )
}

pub fn direct(memory: &impl Memory) -> Split {
    collect_typed(memory, 1024, 65_536)
}

pub fn split(memory: &impl Memory) -> (Split, RadarCopyReport, RadarJob) {
    collect_split(memory, 1024, 65_536)
}

pub fn direct_with(memory: &impl Memory, rows: usize, provinces: usize) -> Split {
    collect_typed(memory, rows, provinces)
}

pub fn split_with(
    memory: &impl Memory,
    rows: usize,
    provinces: usize,
) -> (Split, RadarCopyReport, RadarJob) {
    collect_split(memory, rows, provinces)
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.outcome, direct.outcome, "radar outcome");
    assert_eq!(
        &split.rows[..split.outcome.len()],
        &direct.rows[..direct.outcome.len()],
        "radar rows"
    );
    assert_eq!(split.provinces, direct.provinces, "radar provinces");
}
