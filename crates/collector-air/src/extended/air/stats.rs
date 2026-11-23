use super::*;

pub const AIR_STATS_N: usize = 1;
pub(super) const STATS_MASK: u32 = 0x01;

#[derive(Debug, Default)]
pub struct AirStatsJob {
    pub log: RawLog<AIR_STATS_N>,
    pub game_state: usize,
    pub(super) active: bool,
}

impl AirStatsJob {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AirStatsCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; AIR_STATS_N],
}

#[derive(Debug, Clone, Copy)]
pub struct AirStatsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

pub(super) fn sspan(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine_native::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn stats_pool_u64(log: &RawLog<AIR_STATS_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn stats_pool_u32(log: &RawLog<AIR_STATS_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn copy_stats_history(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    history: usize,
) -> bool {
    use engine_air_history::{
        cloop_history, cloop_history::air_stats, loop_history_record_fields, loop_history_window,
    };
    let vtable = match sspan(log, memory, Some(history), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let expected = match engine_sites::at(base, cloop_history::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if stats_pool_u64(log, vtable) != expected {
        return false;
    }
    let field_count = match sspan(
        log,
        memory,
        history.checked_add(cloop_history::FIELD_COUNT),
        4,
        STATS_MASK,
    ) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    if field_count != air_stats::FIELD_COUNT {
        return false;
    }
    let self_ref = match sspan(
        log,
        memory,
        history.checked_add(cloop_history::SELF_REFERENCE),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, self_ref) != history as u64 {
        return false;
    }
    let window = match sspan(
        log,
        memory,
        history.checked_add(cloop_history::WINDOW),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let window = match usize::try_from(stats_pool_u64(log, window))
        .ok()
        .filter(|&p| p != 0)
    {
        Some(window) => window,
        None => return false,
    };
    let window_vtable = match sspan(log, memory, Some(window), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let expected_window = match engine_sites::at(base, loop_history_window::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if stats_pool_u64(log, window_vtable) != expected_window {
        return false;
    }
    let records_header = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::RECORDS),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[records_header..records_header + 16]);
    let records = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    let capacity = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::CAPACITY),
        4,
        STATS_MASK,
    ) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    let year_slot = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::CURRENT_SLOT),
        4,
        STATS_MASK,
    ) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    let wrapped = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::WRAPPED),
        1,
        STATS_MASK,
    ) {
        Some(start) => log.bytes[start],
        None => return false,
    };
    if capacity as i32 <= 0 || records.count != capacity || year_slot >= capacity || wrapped > 1 {
        return false;
    }
    let owner_back = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::HISTORY),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, owner_back) != history as u64 {
        return false;
    }
    let record_ptr = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::RECORD_FIELDS),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let record = match usize::try_from(stats_pool_u64(log, record_ptr))
        .ok()
        .filter(|&p| p != 0)
    {
        Some(record) => record,
        None => return false,
    };
    let slot_addr = match (year_slot as usize)
        .checked_mul(8)
        .and_then(|o| records.data.checked_add(o))
    {
        Some(addr) => addr,
        None => return false,
    };
    let slot = match sspan(log, memory, Some(slot_addr), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, slot) != record as u64 {
        return false;
    }
    let record_owner = match sspan(
        log,
        memory,
        record.checked_add(loop_history_record_fields::HISTORY),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, record_owner) != history as u64 {
        return false;
    }
    let values_header = match sspan(
        log,
        memory,
        record.checked_add(loop_history_record_fields::VALUES),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    raw.copy_from_slice(&log.bytes[values_header..values_header + 16]);
    let values = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    if values.count != air_stats::FIELD_COUNT {
        return false;
    }
    copy_stats_channels(log, memory, values)
}

pub(super) fn copy_stats_channels(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine_native::Memory,
    values: Array,
) -> bool {
    let lengths = [32, 8, 8, 8, 72, 8, 8, 8, 40];
    let addresses = [0_usize, 10, 17, 24, 36, 51, 58, 65, 77].map(|index| {
        index
            .checked_mul(engine_air_history::loop_history_record_fields::VALUE_STRIDE)
            .and_then(|offset| values.data.checked_add(offset))
    });
    if !log.overflowed
        && addresses.iter().all(Option::is_some)
        && log.spans.capacity() - log.spans.len() >= lengths.len()
        && log
            .bytes
            .len()
            .checked_add(192)
            .is_some_and(|end| end <= log.bytes.capacity() && end <= u32::MAX as usize)
    {
        return log
            .take_scattered(
                memory,
                addresses.map(Option::unwrap),
                lengths,
                [STATS_MASK; 9],
            )
            .is_some_and(|(_, completed)| completed == lengths.len());
    }
    for (address, len) in addresses.into_iter().zip(lengths) {
        if sspan(log, memory, address, len, STATS_MASK).is_none() {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

fn copy_stats_pass(
    job: &mut AirStatsJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; AIR_STATS_N],
) {
    copy_stats_pass_mode(job, memory, base, game_state, ticks);
}

fn copy_stats_pass_mode(
    job: &mut AirStatsJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; AIR_STATS_N],
) {
    let _reads = read_accounting::table(TableId::AirStatistics);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    let started = qpc();
    copy_stats_owners(&mut job.log, memory, base, game_state);
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

fn copy_stats_owners(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
) {
    use engine_native::{ccountry, cstrategic_air, cstrategic_air_manager};
    let countries = match sspan(
        log,
        memory,
        game_state.checked_add(engine_native::cgame_state::COUNTRIES),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match sspan(
        log,
        memory,
        game_state.checked_add(engine_native::cgame_state::COUNTRY_LINKS),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let manager_ptr = match sspan(
        log,
        memory,
        game_state.checked_add(engine_native::cgame_state::STRATEGIC_AIR_MANAGER),
        8,
        STATS_MASK,
    )
    .and_then(|s| {
        usize::try_from(stats_pool_u64(log, s))
            .ok()
            .filter(|&p| p != 0)
    }) {
        Some(manager) => manager,
        None => return,
    };
    if sspan(log, memory, Some(manager_ptr), 8, STATS_MASK).is_none() {
        return;
    }
    let owners_header = match sspan(
        log,
        memory,
        manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries..countries + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links..links + 16]);
    let links_array = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[owners_header..owners_header + 16]);
    let owners = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if owners.count == 0 || countries_array.count < 2 || links_array.count == 0 {
        return;
    }
    for link in 0..links_array.count {
        let addr = match (link as usize)
            .checked_mul(4)
            .and_then(|o| links_array.data.checked_add(o))
        {
            Some(addr) => addr,
            None => return,
        };
        if sspan(log, memory, Some(addr), 4, STATS_MASK).is_none() {
            return;
        }
        if log.overflowed {
            return;
        }
    }
    let country_vtable = match engine_sites::at(base, ccountry::VTABLE_RVA) {
        Some(vtable) => vtable as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let pointer = sspan(log, memory, pointer_addr, 8, STATS_MASK).and_then(|s| {
            usize::try_from(stats_pool_u64(log, s))
                .ok()
                .filter(|&p| p != 0)
        });
        if let Some(pointer) = pointer
            && let Some(vtable) = sspan(log, memory, Some(pointer), 8, STATS_MASK)
            && stats_pool_u64(log, vtable) == country_vtable
        {
            sspan(
                log,
                memory,
                pointer.checked_add(ccountry::ID),
                4,
                STATS_MASK,
            );
        }
        if log.overflowed {
            return;
        }
    }
    let air_vtable = match engine_sites::at(base, cstrategic_air::VTABLE_RVA) {
        Some(vtable) => vtable as u64,
        None => return,
    };
    for slot in 0..owners.count {
        let owner_addr = owners.data.checked_add((slot as usize).saturating_mul(8));
        let owner = match sspan(log, memory, owner_addr, 8, STATS_MASK)
            .and_then(|s| usize::try_from(stats_pool_u64(log, s)).ok())
        {
            Some(owner) => owner,
            None => {
                if log.overflowed {
                    return;
                }
                return;
            }
        };
        if owner == 0 {
            continue;
        }
        let vtable = match sspan(log, memory, Some(owner), 8, STATS_MASK) {
            Some(start) => start,
            None => return,
        };
        if stats_pool_u64(log, vtable) != air_vtable {
            return;
        }
        let owner_field = match sspan(
            log,
            memory,
            owner.checked_add(cstrategic_air::COUNTRY),
            4,
            STATS_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let _ = stats_pool_u32(log, owner_field);
        let table_header = match sspan(
            log,
            memory,
            owner.checked_add(cstrategic_air::HISTORY_TABLE),
            16,
            STATS_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let mut table_raw = [0u8; 16];
        table_raw.copy_from_slice(&log.bytes[table_header..table_header + 16]);
        let table = match Array::from_header(table_raw, 8) {
            Some(array) => array,
            None => continue,
        };
        for first in (0..table.count).step_by(64) {
            let count = (table.count - first).min(64) as usize;
            let Some(address) = table.element(first, 8) else {
                return;
            };
            let Some((start, completed)) =
                log.take_array_chunk(memory, address, 8, count, STATS_MASK)
            else {
                return;
            };
            for index in 0..completed {
                let history = match usize::try_from(stats_pool_u64(log, start + index * 8)).ok() {
                    Some(history) => history,
                    None => return,
                };
                if history != 0 && !copy_stats_history(log, memory, base, history) {
                    return;
                }
                if log.overflowed {
                    return;
                }
            }
            if completed != count {
                return;
            }
        }
        if log.overflowed {
            return;
        }
    }
}

pub fn copy_air_stats_refresh(
    memory: &impl Memory,
    ctx: AirStatsCopyContext,
    job: &mut AirStatsJob,
) -> AirStatsCopyReport {
    if !replay_data::cadence::should_walk(TableId::AirStatistics, ctx.hour, ctx.force_full) {
        job.clear();
        return AirStatsCopyReport::default();
    }
    let mut ticks = [0u64; AIR_STATS_N];
    copy_stats_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    AirStatsCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
    }
}

pub fn decode_air_stats_payload(
    captured: &mut Captured,
    base: usize,
    job: &AirStatsJob,
    decode_ticks: &mut [u64; AIR_STATS_N],
) {
    captured.active[4] = job.active;
    if !job.active {
        captured.fail_air_stats_capture(replay_data::cadence::SKIPPED_REASON);
        return;
    }
    let recorded = match collector_raw::raw::Recorded::build(&job.log) {
        Ok(recorded) => recorded,
        Err(reason) => {
            captured.fail_air_stats_capture(reason);
            return;
        }
    };
    let started = qpc();
    let walked = cloop_history::walk(
        &recorded,
        base,
        job.game_state,
        &mut captured.statistics.rows,
    );
    captured.statistics.outcome = bounded(walked, captured.statistics.rows.len());
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    captured.stats_raw_mode = false;
}
