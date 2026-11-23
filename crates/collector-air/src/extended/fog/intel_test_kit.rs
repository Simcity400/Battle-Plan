use super::{
    Capacities, Captured, IntelCopyContext, IntelCopyReport, IntelJob,
    copy_intel_refresh_with_divisions,
};
use engine_native_memory::{Memory, memory::SliceMemory, walk::Outcome};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
const LTAB: usize = 0x12000;
pub const C0: usize = 0x20000;
pub const DIVISION: usize = 0x50000;
const FLEET: usize = 0x58000;
const TASK_FORCE: usize = 0x60000;
pub const VIEWERS: usize = 0x70000;

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
    use engine_native::{carmy, ccountry, cfleet, cgame_state, ctask_force, cunit};
    use engine_visibility::unit_viewer_entry;
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
    header(&mut bytes, C0 + ccountry::DIVISIONS, 0x40000, 1);
    put64(&mut bytes, 0x40000, DIVISION as u64);
    header(&mut bytes, C0 + ccountry::FLEETS, 0x41000, 1);
    put64(&mut bytes, 0x41000, FLEET as u64);
    put64(&mut bytes, FLEET, cfleet::VTABLE_RVA.0 as u64);
    header(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x42000, 1);
    put64(&mut bytes, 0x42000, TASK_FORCE as u64);
    put64(&mut bytes, DIVISION, carmy::VTABLE_RVA.0 as u64);
    put64(&mut bytes, DIVISION + cunit::HANDLE, (7u64 << 32) | 3);
    header(&mut bytes, DIVISION + cunit::VIEWER_DATA, VIEWERS, 2);
    for (index, (viewer, level)) in [(1u32, 3u8), (2, 255)].into_iter().enumerate() {
        put32(
            &mut bytes,
            VIEWERS + index * unit_viewer_entry::STRIDE,
            viewer,
        );
        bytes[VIEWERS + index * unit_viewer_entry::STRIDE + unit_viewer_entry::LEVEL] = level;
    }
    put64(&mut bytes, TASK_FORCE, ctask_force::VTABLE_RVA.0 as u64);
    put64(&mut bytes, TASK_FORCE + cunit::HANDLE, u64::MAX);
    header(&mut bytes, TASK_FORCE + cunit::VIEWER_DATA, 0, 0);
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub struct Split {
    pub outcome: Outcome,
    pub rows: Vec<engine_visibility::unit_viewer_entry::Row>,
}

fn collect_typed(memory: &impl Memory, cap: usize) -> Split {
    use engine_visibility::unit_viewer_entry;
    let capacities = Capacities {
        rows: [4, 4, 4, 4, cap, 4, 4],
        radar_provinces: 8,
        area_provinces: 8,
        wings: 8,
    };
    let mut captured = Captured::with_capacities(&capacities);
    let walked = unit_viewer_entry::walk(memory, BASE, GAME_STATE, &mut captured.intel.rows);
    captured.intel.outcome = super::bounded(walked, captured.intel.rows.len());
    Split {
        outcome: captured.intel.outcome,
        rows: captured.intel.rows.clone(),
    }
}

fn collect_split(memory: &impl Memory, cap: usize) -> (Split, IntelCopyReport, IntelJob) {
    let ctx = IntelCopyContext {
        base: BASE,
        game_state: GAME_STATE,
        row_limit: cap,
    };
    let capacities = Capacities {
        rows: [4, 4, 4, 4, cap, 4, 4],
        radar_provinces: 8,
        area_provinces: 8,
        wings: 8,
    };
    let mut captured = Captured::with_capacities(&capacities);
    let mut first = IntelJob::with_storage(cap, 0);
    let report = copy_intel_refresh_with_divisions(memory, ctx, &mut first, None);
    assert!(report.raw_ok);
    let mut ticks = [0u64; 1];
    captured.decode_intel_payload(&first, &mut ticks);
    (
        Split {
            outcome: captured.intel.outcome,
            rows: captured.intel.rows.clone(),
        },
        report,
        first,
    )
}

pub fn direct(memory: &impl Memory) -> Split {
    collect_typed(memory, 32_768)
}

pub fn split(memory: &impl Memory) -> (Split, IntelCopyReport, IntelJob) {
    collect_split(memory, 32_768)
}

pub fn direct_with(memory: &impl Memory, cap: usize) -> Split {
    collect_typed(memory, cap)
}

pub fn split_with(memory: &impl Memory, cap: usize) -> (Split, IntelCopyReport, IntelJob) {
    collect_split(memory, cap)
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.outcome, direct.outcome, "intel outcome");
    assert_eq!(
        &split.rows[..split.outcome.len()],
        &direct.rows[..direct.outcome.len()],
        "intel rows"
    );
}

pub fn assert_paths_match_direct(split: &Split, direct: &Split) {
    use engine_native::ctask_force;
    let is_fleet = |unit: u64| (unit & 0xffff_ffff) == u64::from(ctask_force::DEFAULT_HANDLE_TYPE);
    for fleet in [false, true] {
        let split_path: Vec<_> = split.rows[..split.outcome.len()]
            .iter()
            .filter(|row| is_fleet(row.unit) == fleet)
            .collect();
        let direct_path: Vec<_> = direct.rows[..direct.outcome.len()]
            .iter()
            .filter(|row| is_fleet(row.unit) == fleet)
            .collect();
        assert_eq!(split_path, direct_path, "intel path fleet={fleet}");
    }
}
