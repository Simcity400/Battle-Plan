use super::{AirStatsCopyContext, AirStatsCopyReport, AirStatsJob, Capacities, Captured};
use engine_native_memory::{Memory, memory::SliceMemory, walk::Outcome};

pub const BASE: usize = 0x1400_0000;
pub const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
const LTAB: usize = 0x12000;
const COUNTRY: usize = 0x20000;
const MANAGER: usize = 0x30000;
const OWNER_TAB: usize = 0x31000;
const AIR: usize = 0x40000;
const HIST_TAB: usize = 0x41000;
const HISTORY1: usize = 0x50000;
const HISTORY2: usize = 0x50200;
const WINDOW1: usize = 0x60000;
const WINDOW2: usize = 0x60200;
const SLOTS1: usize = 0x61000;
const SLOTS2: usize = 0x61200;
const RECORD1: usize = 0x70000;
const RECORD2: usize = 0x70200;
const VALUES1: usize = 0x80000;
const VALUES2: usize = 0x81000;

fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}

fn history(
    bytes: &mut [u8],
    history: usize,
    window: usize,
    slots: usize,
    record: usize,
    values: usize,
    seed: i64,
) {
    use engine_air_history::{
        cloop_history, cloop_history::air_stats, loop_history_record_fields, loop_history_window,
    };
    p64(bytes, history, (BASE + cloop_history::VTABLE_RVA.0) as u64);
    p32(
        bytes,
        history + cloop_history::FIELD_COUNT,
        air_stats::FIELD_COUNT,
    );
    p64(
        bytes,
        history + cloop_history::SELF_REFERENCE,
        history as u64,
    );
    p64(bytes, history + cloop_history::WINDOW, window as u64);
    p64(
        bytes,
        window,
        (BASE + loop_history_window::VTABLE_RVA.0) as u64,
    );
    header(bytes, window + loop_history_window::RECORDS, slots, 5);
    p32(bytes, window + loop_history_window::CAPACITY, 5);
    p32(bytes, window + loop_history_window::CURRENT_SLOT, 2);
    bytes[window + loop_history_window::WRAPPED] = 1;
    p64(bytes, window + loop_history_window::HISTORY, history as u64);
    p64(
        bytes,
        window + loop_history_window::RECORD_FIELDS,
        record as u64,
    );
    for slot in 0..5 {
        p64(bytes, slots + slot * 8, 0);
    }
    p64(bytes, slots + 2 * 8, record as u64);
    p64(
        bytes,
        record + loop_history_record_fields::HISTORY,
        history as u64,
    );
    header(
        bytes,
        record + loop_history_record_fields::VALUES,
        values,
        air_stats::FIELD_COUNT,
    );
    for channel in 0..air_stats::FIELD_COUNT as usize {
        p64(bytes, values + channel * 8, (seed + channel as i64) as u64);
    }
}

pub fn fixture() -> Vec<u8> {
    use engine_native::{ccountry, cgame_state};
    use engine_native_air::{cstrategic_air, cstrategic_air_manager};
    let mut bytes = vec![0; 0x82000];
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 3);
    p64(&mut bytes, CTAB, 0);
    p64(&mut bytes, CTAB + 8, COUNTRY as u64);
    p64(&mut bytes, CTAB + 16, COUNTRY as u64);
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
    p32(&mut bytes, LTAB, 0);
    p32(&mut bytes, LTAB + 4, 1);
    p32(&mut bytes, LTAB + 8, 1);
    p32(&mut bytes, LTAB + 12, 1);
    p64(&mut bytes, COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
    p32(&mut bytes, COUNTRY + ccountry::ID, 3);
    p64(
        &mut bytes,
        GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    p64(
        &mut bytes,
        MANAGER,
        (BASE + cstrategic_air_manager::VTABLE_RVA.0) as u64,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        OWNER_TAB,
        3,
    );
    p64(&mut bytes, OWNER_TAB, 0);
    p64(&mut bytes, OWNER_TAB + 8, AIR as u64);
    p64(&mut bytes, OWNER_TAB + 16, 0);
    p64(
        &mut bytes,
        AIR,
        (BASE + cstrategic_air::VTABLE_RVA.0) as u64,
    );
    p32(&mut bytes, AIR + cstrategic_air::COUNTRY, 2);
    header(&mut bytes, AIR + cstrategic_air::HISTORY_TABLE, HIST_TAB, 2);
    p64(&mut bytes, HIST_TAB, HISTORY1 as u64);
    p64(&mut bytes, HIST_TAB + 8, HISTORY2 as u64);
    history(
        &mut bytes, HISTORY1, WINDOW1, SLOTS1, RECORD1, VALUES1, 1000,
    );
    history(
        &mut bytes, HISTORY2, WINDOW2, SLOTS2, RECORD2, VALUES2, 2000,
    );
    bytes
}

pub fn add_reserved_alias_and_null_slot(bytes: &mut [u8]) {
    use engine_native::{ccountry, cgame_state};
    use engine_native_air::cstrategic_air_manager;
    const RESERVED: usize = 0x20100;
    header(bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
    p64(bytes, CTAB, RESERVED as u64);
    p64(bytes, CTAB + 8, RESERVED as u64);
    p64(bytes, CTAB + 16, 0);
    p64(bytes, CTAB + 24, COUNTRY as u64);
    p64(bytes, RESERVED, (BASE + ccountry::VTABLE_RVA.0) as u64);
    p32(bytes, RESERVED + ccountry::ID, 0);
    p32(bytes, LTAB + 4, 0);
    p32(bytes, LTAB + 8, 3);
    p32(bytes, LTAB + 12, 3);
    header(
        bytes,
        MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        OWNER_TAB,
        4,
    );
    p64(bytes, OWNER_TAB + 8, 0);
    p64(bytes, OWNER_TAB + 24, AIR as u64);
}

pub fn set_first_value(bytes: &mut [u8], value: i64) {
    p64(bytes, VALUES1, value as u64);
}

pub fn set_yearly_capacity(bytes: &mut Vec<u8>, capacity: u32) {
    use engine_air_history::loop_history_window;
    for (window, record) in [(WINDOW1, RECORD1), (WINDOW2, RECORD2)] {
        let slots = bytes.len();
        bytes.resize(slots + capacity as usize * 8, 0);
        header(
            bytes,
            window + loop_history_window::RECORDS,
            slots,
            capacity,
        );
        p32(bytes, window + loop_history_window::CAPACITY, capacity);
        p32(
            bytes,
            window + loop_history_window::CURRENT_SLOT,
            capacity - 1,
        );
        p64(bytes, slots + (capacity as usize - 1) * 8, record as u64);
    }
}

pub fn mod_fixture(count: usize) -> Vec<u8> {
    let mut bytes = fixture();
    let data = bytes.len();
    bytes.resize(data + count * 8, 0);
    header(
        &mut bytes,
        AIR + engine_native_air::cstrategic_air::HISTORY_TABLE,
        data,
        count as u32,
    );
    for index in 0..count {
        p64(&mut bytes, data + index * 8, HISTORY1 as u64);
    }
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub struct Split {
    pub outcome: Outcome,
    pub rows: Vec<engine_air_history::cloop_history::Row>,
}

fn collect_typed(memory: &impl Memory, capacity: &Capacities) -> Split {
    use engine_air_history::cloop_history;
    let mut captured = Captured::with_capacities(capacity);
    let walked = cloop_history::walk(memory, BASE, GAME_STATE, &mut captured.statistics.rows);
    captured.statistics.outcome = super::bounded(walked, captured.statistics.rows.len());
    Split {
        outcome: captured.statistics.outcome,
        rows: captured.statistics.rows.clone(),
    }
}

pub fn direct(memory: &impl Memory, capacity: &Capacities) -> Split {
    collect_typed(memory, capacity)
}

pub fn split(
    memory: &impl Memory,
    capacity: &Capacities,
) -> (Split, AirStatsCopyReport, AirStatsJob) {
    let ctx = AirStatsCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut first = AirStatsJob::fixture();
    let report = super::copy_air_stats_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut captured = Captured::with_capacities(capacity);
    let mut ticks = [0u64; 1];
    super::decode_air_stats_payload(&mut captured, BASE, &first, &mut ticks);
    (
        Split {
            outcome: captured.statistics.outcome,
            rows: captured.statistics.rows.clone(),
        },
        report,
        first,
    )
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.outcome, direct.outcome, "statistics outcome");
    assert_eq!(split.rows, direct.rows, "statistics rows");
}

pub fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
    assert_eq!(
        captured.statistics.outcome, direct.outcome,
        "statistics outcome"
    );
    assert_eq!(
        &captured.statistics.rows[..captured.statistics.outcome.len()],
        &direct.rows[..direct.outcome.len()],
        "statistics rows"
    );
}
