use super::*;

const SUPREMACY_MASK: u32 = 0x01;

const SUPREMACY_MIRROR_BATCH: usize = 32;

#[derive(Debug, Default)]
pub struct SupremacyJob {
    pub log: RawLog<SUPREMACY_N>,
    pub game_state: usize,
    pub(super) active: bool,
}

impl SupremacyJob {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn fixture() -> Self {
        Self {
            log: RawLog::new(1 << 12, 1 << 20),
            game_state: 0,
            active: false,
        }
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
pub struct SupremacyCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; SUPREMACY_N],
}

#[derive(Debug, Clone, Copy)]
pub struct SupremacyCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn supremacy_pool_u64(log: &RawLog<SUPREMACY_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn sspan(
    log: &mut RawLog<SUPREMACY_N>,
    memory: &impl engine_native_memory::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn copy_supremacy_pass(
    job: &mut SupremacyJob,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; SUPREMACY_N],
) {
    copy_supremacy_pass_mode(job, memory, base, game_state, ticks);
}

fn copy_supremacy_pass_mode(
    job: &mut SupremacyJob,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; SUPREMACY_N],
) {
    let _reads = read_accounting::table(TableId::NavalSupremacy);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    job.log.has_bulk = true;
    let started = qpc();
    copy_supremacy_tables(job, memory, base, game_state);
    ticks[0] += elapsed(started);
}

fn copy_supremacy_tables(
    job: &mut SupremacyJob,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
) {
    let log = &mut job.log;
    let regions_header = match sspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STRATEGIC_REGIONS),
        16,
        SUPREMACY_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[regions_header..regions_header + 16]);
    let regions = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if regions.count == 0 {
        return;
    }
    let region_vtable = match engine_sites::at(base, cstrategic_region::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let secondary_vtable = match engine_sites::at(base, cstrategic_region::SECONDARY_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let dominance_vtable = match engine_sites::at(base, cnaval_region_dominance::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let values_vtable = match engine_sites::at(base, cdominance_values::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    for ordinal in 1..regions.count {
        let addr = (ordinal as usize)
            .checked_mul(8)
            .and_then(|offset| regions.data.checked_add(offset));
        let pointer_start = match sspan(log, memory, addr, 8, SUPREMACY_MASK) {
            Some(start) => start,
            None => return,
        };
        let pointer = match usize::try_from(supremacy_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtables = match sspan(log, memory, Some(pointer), 16, SUPREMACY_MASK) {
            Some(start) => start,
            None => return,
        };
        let identity_len = cstrategic_region::RUNTIME_SLOT - cstrategic_region::ID + 4;
        let identity = match sspan(
            log,
            memory,
            pointer.checked_add(cstrategic_region::ID),
            identity_len,
            SUPREMACY_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let dominance_start = match sspan(
            log,
            memory,
            pointer.checked_add(cstrategic_region::NAVAL_DOMINANCE),
            8,
            SUPREMACY_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        if supremacy_pool_u64(log, vtables) != region_vtable
            || supremacy_pool_u64(log, vtables + 8) != secondary_vtable
        {
            return;
        }
        let region_slot = u32::from_le_bytes(
            log.bytes[identity + identity_len - 4..identity + identity_len]
                .try_into()
                .unwrap(),
        );
        if region_slot != ordinal {
            return;
        }
        let dominance = match usize::try_from(supremacy_pool_u64(log, dominance_start))
            .ok()
            .filter(|&address| address != 0)
        {
            Some(dominance) => dominance,
            None => return,
        };
        let owner = match sspan(log, memory, Some(dominance), 16, SUPREMACY_MASK) {
            Some(start) => start,
            None => return,
        };
        if supremacy_pool_u64(log, owner) != dominance_vtable
            || supremacy_pool_u64(log, owner + cnaval_region_dominance::REGION) != pointer as u64
        {
            return;
        }
        let entries_header = match sspan(
            log,
            memory,
            dominance.checked_add(cnaval_region_dominance::ENTRIES),
            16,
            SUPREMACY_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        raw.copy_from_slice(&log.bytes[entries_header..entries_header + 16]);
        let entries = match Array::from_header(raw, naval_dominance_entry::STRIDE) {
            Some(array) => array,
            None => return,
        };
        if entries.count != 0 {
            let bulk_start = log.take_bulk(
                memory,
                entries.data,
                entries.count as usize * naval_dominance_entry::STRIDE,
                SUPREMACY_MASK,
            );
            let Some(bulk_start) = bulk_start else {
                return;
            };
            for index in 0..entries.count as usize {
                let at = bulk_start + index * naval_dominance_entry::STRIDE;
                let bytes = match log.bytes.get(at..at + naval_dominance_entry::STRIDE) {
                    Some(bytes) => bytes,
                    None => return,
                };
                let country = u32::from_le_bytes(
                    bytes[naval_dominance_entry::COUNTRY_KEY
                        ..naval_dominance_entry::COUNTRY_KEY + 4]
                        .try_into()
                        .unwrap(),
                );
                let object = &bytes[naval_dominance_entry::VALUES..];
                if u64::from_le_bytes(object[..8].try_into().unwrap()) != values_vtable
                    || u32::from_le_bytes(
                        object[cdominance_values::COUNTRY..cdominance_values::COUNTRY + 4]
                            .try_into()
                            .unwrap(),
                    ) != country
                    || u64::from_le_bytes(
                        object[cdominance_values::REGION..cdominance_values::REGION + 8]
                            .try_into()
                            .unwrap(),
                    ) != pointer as u64
                {
                    return;
                }
            }
        }
        if log.overflowed {
            return;
        }
        let _ = SUPREMACY_MIRROR_BATCH;
    }
}

pub fn copy_supremacy_refresh(
    memory: &impl Memory,
    ctx: SupremacyCopyContext,
    job: &mut SupremacyJob,
) -> SupremacyCopyReport {
    if !replay_data::cadence::should_walk(TableId::NavalSupremacy, ctx.hour, ctx.force_full) {
        job.clear();
        return SupremacyCopyReport::default();
    }
    let mut ticks = [0u64; SUPREMACY_N];
    copy_supremacy_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    SupremacyCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
    }
}

impl Captured {
    pub fn fail_supremacy_capture(&mut self, reason: &'static str) {
        self.supremacy.outcome = failed(self.active[0], reason);
        self.supremacy_raw_mode = false;
    }

    pub fn decode_supremacy_payload(
        &mut self,
        base: usize,
        job: &SupremacyJob,
        decode_ticks: &mut [u64; SUPREMACY_N],
    ) {
        self.active[0] = job.active;
        if !job.active {
            self.fail_supremacy_capture(replay_data::cadence::SKIPPED_REASON);
            return;
        }
        let recorded = match collector_raw::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_supremacy_capture(reason);
                return;
            }
        };
        let started = qpc();
        let walked = cnaval_region_dominance::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.supremacy.rows,
        );
        self.supremacy.outcome = bounded(walked, self.supremacy.rows.len());
        decode_ticks[0] += elapsed(started);
        self.supremacy_raw_mode = false;
    }
}
