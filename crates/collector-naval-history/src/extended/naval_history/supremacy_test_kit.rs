use super::{Capacities, Captured, SupremacyCopyContext, SupremacyCopyReport, SupremacyJob};
use engine_native::{Memory, memory::SliceMemory, walk::Outcome};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x1000;
const REGIONS: usize = 0x1400;
pub const REGION_OBJECT: usize = 0x1500;
pub const DOMINANCE: usize = 0x1800;
const ROWS: usize = 0x1900;

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
    use engine_native::{cgame_state, cstrategic_region};
    use engine_naval_history::{cdominance_values, cnaval_region_dominance, naval_dominance_entry};
    let mut bytes = vec![0; 0x4000];
    header(
        &mut bytes,
        GAME_STATE + cgame_state::STRATEGIC_REGIONS,
        REGIONS,
        3,
    );
    put64(&mut bytes, REGIONS, 0);
    put64(&mut bytes, REGIONS + 8, REGION_OBJECT as u64);
    put64(&mut bytes, REGIONS + 16, 0);
    put64(
        &mut bytes,
        REGION_OBJECT,
        cstrategic_region::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut bytes,
        REGION_OBJECT + cstrategic_region::SECONDARY,
        cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64,
    );
    put32(
        &mut bytes,
        REGION_OBJECT + cstrategic_region::ID,
        0x8765_4321,
    );
    put32(
        &mut bytes,
        REGION_OBJECT + cstrategic_region::RUNTIME_SLOT,
        1,
    );
    put64(
        &mut bytes,
        REGION_OBJECT + cstrategic_region::NAVAL_DOMINANCE,
        DOMINANCE as u64,
    );
    put64(
        &mut bytes,
        DOMINANCE,
        cnaval_region_dominance::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut bytes,
        DOMINANCE + cnaval_region_dominance::REGION,
        REGION_OBJECT as u64,
    );
    header(
        &mut bytes,
        DOMINANCE + cnaval_region_dominance::ENTRIES,
        ROWS,
        2,
    );
    for (index, country) in [u32::MAX, 1].into_iter().enumerate() {
        let outer = ROWS + index * naval_dominance_entry::STRIDE;
        let object = outer + naval_dominance_entry::VALUES;
        put32(
            &mut bytes,
            outer + naval_dominance_entry::COUNTRY_KEY,
            country,
        );
        put64(&mut bytes, object, cdominance_values::VTABLE_RVA.0 as u64);
        put32(&mut bytes, object + cdominance_values::COUNTRY, country);
        put64(
            &mut bytes,
            object + cdominance_values::REGION,
            REGION_OBJECT as u64,
        );
        put64(
            &mut bytes,
            object + cdominance_values::CURRENT,
            (1000 + index) as u64,
        );
        put64(
            &mut bytes,
            object + cdominance_values::BASE_TARGET,
            (2000 + index) as u64,
        );
    }
    put64(
        &mut bytes,
        ROWS + 2 * naval_dominance_entry::STRIDE,
        u64::MAX,
    );
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub fn mod_fixture(count: usize) -> Vec<u8> {
    let mut bytes = fixture();
    let entry = bytes[ROWS..ROWS + 0x50].to_vec();
    let data = bytes.len();
    bytes.resize(data + count * 0x50, 0);
    header(
        &mut bytes,
        DOMINANCE + engine_naval_history::cnaval_region_dominance::ENTRIES,
        data,
        count as u32,
    );
    for row in bytes[data..].chunks_exact_mut(0x50) {
        row.copy_from_slice(&entry);
    }
    bytes
}

pub struct Split {
    pub outcome: Outcome,
    pub rows: Vec<engine_naval_history::cnaval_region_dominance::Row>,
}

fn collect_typed(memory: &impl Memory, cap: usize) -> Split {
    use engine_naval_history::cnaval_region_dominance;
    let mut capacities = Capacities::fixture();
    capacities.rows[0] = cap;
    let mut captured = Captured::with_capacities(&capacities);
    let walked =
        cnaval_region_dominance::walk(memory, BASE, GAME_STATE, &mut captured.supremacy.rows);
    captured.supremacy.outcome = super::bounded(walked, captured.supremacy.rows.len());
    Split {
        outcome: captured.supremacy.outcome,
        rows: captured.supremacy.rows.clone(),
    }
}

fn collect_split(memory: &impl Memory, cap: usize) -> (Split, SupremacyCopyReport, SupremacyJob) {
    let ctx = SupremacyCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut capacities = Capacities::fixture();
    capacities.rows[0] = cap;
    let mut captured = Captured::with_capacities(&capacities);
    let mut first = SupremacyJob::fixture();
    let report = super::copy_supremacy_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut ticks = [0u64; 1];
    captured.decode_supremacy_payload(BASE, &first, &mut ticks);
    (
        Split {
            outcome: captured.supremacy.outcome,
            rows: captured.supremacy.rows.clone(),
        },
        report,
        first,
    )
}

pub fn direct(memory: &impl Memory) -> Split {
    collect_typed(memory, 4096)
}

pub fn split(memory: &impl Memory) -> (Split, SupremacyCopyReport, SupremacyJob) {
    collect_split(memory, 4096)
}

pub fn direct_with(memory: &impl Memory, cap: usize) -> Split {
    collect_typed(memory, cap)
}

pub fn split_with(memory: &impl Memory, cap: usize) -> (Split, SupremacyCopyReport, SupremacyJob) {
    collect_split(memory, cap)
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.outcome, direct.outcome, "supremacy outcome");
    assert_eq!(split.rows, direct.rows, "supremacy rows");
}
