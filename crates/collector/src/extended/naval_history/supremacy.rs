use super::*;

// ===========================================================================
// Hourly supremacy split (NavalSupremacy).
//
// The game thread copies raw bytes only: the strategic-region roster, per
// region identity with its dominance container, and every dominance value
// vector. No row is decoded on the game thread. The single FIFO decode
// worker replays `cnaval_region_dominance::walk` against the recorded spans,
// so rows, ordinals, skips, truncation counts and failure reasons are
// identical by construction (see `crate::raw` for the mirror rules).
//
// The value vectors are cap-dependent (the typed walk reads a
// capacity-truncated prefix in 32-entry chunks), so they are bulk spans:
// the mirror takes each whole value vector once and the replay serves any
// enclosed chunk from it. Every fault fails the whole table in the typed
// walk, so bulk granularity is exact here: rows never publish past a fault.
// ===========================================================================

const SUPREMACY_MASK: u32 = 0x01;

/// Chunking of the typed walk's value reads (informational: bulk spans
/// serve any enclosed chunk regardless of chunking).
const SUPREMACY_MIRROR_BATCH: usize = 32;

/// One hour of raw supremacy payload. Flat pools recycle capacities and the
/// steady state reuses storage. Worker logs grow before each original request;
/// decode grows from the complete accepted payload.
#[derive(Debug, Default)]
pub(crate) struct SupremacyJob {
    pub log: RawLog<SUPREMACY_N>,
    pub game_state: usize,
    pub(super) active: bool,
}

impl SupremacyJob {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(super) fn fixture() -> Self {
        Self {
            // Explicit raw storage for fixed synthetic fault/read fixtures.
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
pub(crate) struct SupremacyCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; SUPREMACY_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SupremacyCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn supremacy_pool_u64(log: &RawLog<SUPREMACY_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn sspan(
    log: &mut RawLog<SUPREMACY_N>,
    memory: &impl engine_native::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// One structural copy pass: region roster, per-region identity with its
/// dominance container, and every value vector as one bulk span. The job is
/// cleared on entry so pooled refreshes never accumulate spans across hours.
fn copy_supremacy_pass(
    job: &mut SupremacyJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; SUPREMACY_N],
) {
    copy_supremacy_pass_mode(job, memory, base, game_state, ticks);
}

fn copy_supremacy_pass_mode(
    job: &mut SupremacyJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; SUPREMACY_N],
) {
    let _reads = read_accounting::table(TableId::NavalSupremacy);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    // Value vectors are bulk spans (see above); the flag is set up front so
    // owned replay serves enclosed chunks identically.
    job.log.has_bulk = true;
    let started = qpc();
    copy_supremacy_tables(job, memory, base, game_state);
    ticks[0] += elapsed(started);
}

fn copy_supremacy_tables(
    job: &mut SupremacyJob,
    memory: &impl engine_native::Memory,
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
        // copy_scattered triple: vtables word, identity words, dominance word.
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
            // Validate every staged value locally, mirroring the walk: any
            // invalid value fails the whole table there.
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

/// Acquire one owned hourly supremacy payload.
pub(crate) fn copy_supremacy_refresh(
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
    /// Finish an accepted failure after independent capture without live retry.
    pub(crate) fn fail_supremacy_capture(&mut self, reason: &'static str) {
        self.supremacy.outcome = failed(self.active[0], reason);
        self.supremacy_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw supremacy payload: replays the
    /// supremacy walk against the recorded spans into the pooled buffers,
    /// with the identical bounded/capacity handling the game thread applies.
    /// Installs identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_supremacy_payload(
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
        let recorded = match crate::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_supremacy_capture(reason);
                return;
            }
        };
        let started = qpc();
        let walked =
            cnaval_region_dominance::walk(&recorded, base, job.game_state, &mut self.supremacy.rows);
        self.supremacy.outcome = bounded(walked, self.supremacy.rows.len());
        decode_ticks[0] += elapsed(started);
        self.supremacy_raw_mode = false;
    }
}
