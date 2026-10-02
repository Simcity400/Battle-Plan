use super::*;

/// Acquire one owned payload and report its per-table costs.
pub(crate) fn copy_economy_rest_refresh(
    memory: &impl Memory,
    ctx: EconomyRestCopyContext,
    job: &mut EconomyRestJob,
) -> EconomyRestCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    job.resources_active = ctx.resources_active;
    job.definitions_slot = ctx.definitions_slot;
    job.other_active = ctx.other_active;
    // Active resources must have capacity before any raw source fault is consumed.
    if ctx.resources_active
        && !job
            .resources
            .prepare(ctx.resource_rows, ctx.resource_scratch)
    {
        return EconomyRestCopyReport::default();
    }
    copy_economy_rest_pass(
        memory,
        ctx.base,
        ctx.game_state,
        ctx.other_active,
        &mut job.log,
    );
    let mut ticks = [0; N];
    ticks[2] = elapsed(started);
    let overflow = job.log.overflowed;
    let fault = job.log.fault.iter().any(|&fault| fault);
    if overflow && fault {
        job.staging_failure = Some("economy staging unavailable after source read failure");
    } else if ctx.resources_active
        && !overflow
        && job
            .log
            .spans
            .iter()
            .any(|span| !span.ok && span.mask == M_E_CENSUS)
    {
        // Failed shared input must not be retried by the typed resource group.
        job.resources.outcomes = [Some(Outcome::Unavailable {
            reason: "economy shared country census unreadable",
        }); 3];
    } else if ctx.resources_active && !overflow {
        acquire_resource_group(memory, ctx, &mut job.resources, &mut ticks);
    }
    EconomyRestCopyReport {
        raw_ok: !overflow || job.staging_failure.is_some(),
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

fn install_resource_rows<R: Copy>(
    output: &mut [R],
    source: &[R],
    outcome: Option<Outcome>,
    granted: usize,
) -> Outcome {
    let Some(outcome) = outcome else {
        return Outcome::Unavailable {
            reason: "economy resource payload missing",
        };
    };
    if output.len() != granted || outcome.len() > granted || source.len() < granted {
        return Outcome::Unavailable {
            reason: "economy resource payload capacity changed",
        };
    }
    output[..outcome.len()].copy_from_slice(&source[..outcome.len()]);
    outcome
}

fn acquire_resource_group(
    memory: &impl Memory,
    ctx: EconomyRestCopyContext,
    owned: &mut OwnedResources,
    ticks: &mut [u64; N],
) {
    let mut reads = read_accounting::table(TABLES[6]);
    let started = qpc();
    let definitions = &mut owned.definitions[..ctx.resource_rows[0]];
    let outcome = cstrategic_resource::walk(memory, ctx.base, definitions);
    owned.outcomes[0] = Some(outcome);
    ticks[6] = elapsed(started);
    let descriptors = match outcome {
        Outcome::Rows { len } => definitions.get(..len),
        _ => None,
    };
    let Some(descriptors) = descriptors else {
        owned.outcomes[1] = Some(unavailable_descriptors());
        owned.outcomes[2] = Some(unavailable_descriptors());
        return;
    };
    reads.select(TABLES[7]);
    let started = qpc();
    owned.outcomes[1] = Some(ccountry_resources::walk(
        memory,
        ctx.base,
        ctx.game_state,
        descriptors,
        &mut owned.resources[..ctx.resource_rows[1]],
    ));
    ticks[7] = elapsed(started);
    reads.select(TABLES[8]);
    let started = qpc();
    owned.outcomes[2] = Some(cresource_exchange::walk(
        memory,
        ctx.base,
        ctx.game_state,
        descriptors,
        &mut owned.exchanges[..ctx.resource_rows[2]],
        &mut owned.scratch[..ctx.resource_scratch],
    ));
    ticks[8] = elapsed(started);
}

impl Captured {
    /// Interpret eight indexed raw tables and install three owned native tables.
    /// Resource rows are already acquired in dependency order; their
    /// installation performs no native reads, index lookup or replayed walk.
    /// All rows, arenas and truncation outcomes reach the same FIFO
    /// handoff. Clears `rest_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_economy_rest_payload(
        &mut self,
        base: usize,
        job: &EconomyRestJob,
        decode_ticks: &mut [u64; N],
    ) {
        self.resources_active = job.resources_active;
        self.definitions_slot = job.definitions_slot;
        self.other_active = job.other_active;
        self.mark_inactive();
        if let Some(reason) = job.staging_failure {
            self.fail_economy_rest_capture(reason);
            return;
        }
        let recorded = crate::raw::Recorded::build(&job.log);
        if self.other_active {
            let started = qpc();
            self.capture_fuel(&recorded, base, job.game_state);
            decode_ticks[0] += elapsed(started);
            let started = qpc();
            self.experience.outcome = bounded(
                ccountry_experience_status::walk(
                    &recorded,
                    base,
                    job.game_state,
                    &mut self.experience.rows,
                ),
                self.experience.rows.len(),
            );
            decode_ticks[1] += elapsed(started);
        }
        let started = qpc();
        self.supply.outcome = bounded(
            sprovince_supply_data::walk(&recorded, base, job.game_state, &mut self.supply.rows),
            self.supply.rows.len(),
        );
        decode_ticks[2] += elapsed(started);
        if self.other_active {
            let started = qpc();
            self.hubs.outcome = bounded(
                ccountry_supply_system::walk(&recorded, base, job.game_state, &mut self.hubs.rows),
                self.hubs.rows.len(),
            );
            self.disrupted.outcome = bounded(
                disrupted_supply_bucket::walk(
                    &recorded,
                    base,
                    job.game_state,
                    &mut self.disrupted.rows,
                ),
                self.disrupted.rows.len(),
            );
            decode_ticks[3] += elapsed(started);
            decode_ticks[4] += 0;
            let started = qpc();
            let first = foreign_homebase_node::walk(
                &recorded,
                base,
                job.game_state,
                &mut self.foreign.rows,
                &mut self.sources.first,
            );
            self.foreign.outcome = bounded(first.outcome, self.foreign.rows.len());
            self.sources.len = first.bytes_len;
            self.sources.required = first.required_bytes;
            if first.required_bytes < first.bytes_len
                || !valid_sources(
                    self.foreign.outcome,
                    &self.foreign.rows,
                    &self.sources.first,
                    first.bytes_len,
                )
            {
                self.foreign.outcome = invalid_sources();
            }
            decode_ticks[5] += elapsed(started);
        }
        if self.resources_active {
            let started = qpc();
            self.definitions.outcome = install_resource_rows(
                &mut self.definitions.rows,
                &job.resources.definitions,
                job.resources.outcomes[0],
                job.resources.required[0],
            );
            decode_ticks[6] += elapsed(started);
            let started = qpc();
            let valid_definitions = matches!(self.definitions.outcome, Outcome::Rows { .. });
            self.resources.outcome = if valid_definitions {
                install_resource_rows(
                    &mut self.resources.rows,
                    &job.resources.resources,
                    job.resources.outcomes[1],
                    job.resources.required[1],
                )
            } else {
                unavailable_descriptors()
            };
            decode_ticks[7] += elapsed(started);
            let started = qpc();
            self.exchanges.outcome = if valid_definitions {
                install_resource_rows(
                    &mut self.exchanges.rows,
                    &job.resources.exchanges,
                    job.resources.outcomes[2],
                    job.resources.required[2],
                )
            } else {
                unavailable_descriptors()
            };
            decode_ticks[8] += elapsed(started);
            self.hide_off_slot_census();
        }
        if self.other_active {
            let started = qpc();
            self.purchases.outcome = bounded(
                cpurchase_contract::walk(
                    &recorded,
                    base,
                    job.game_state,
                    &mut self.purchases.rows,
                    &mut self.purchase_scratch,
                ),
                self.purchases.rows.len(),
            );
            decode_ticks[9] += elapsed(started);
        }
        self.rest_raw_mode = false;
    }

    /// Finish an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_economy_rest_capture(&mut self, reason: &'static str) {
        self.fuel.outcome = Outcome::Unavailable { reason };
        self.experience.outcome = Outcome::Unavailable { reason };
        self.supply.outcome = Outcome::Unavailable { reason };
        self.hubs.outcome = Outcome::Unavailable { reason };
        self.disrupted.outcome = Outcome::Unavailable { reason };
        self.foreign.outcome = Outcome::Unavailable { reason };
        self.definitions.outcome = Outcome::Unavailable { reason };
        self.resources.outcome = Outcome::Unavailable { reason };
        self.exchanges.outcome = Outcome::Unavailable { reason };
        self.purchases.outcome = Outcome::Unavailable { reason };
        self.fuel_consumers.outcome = Outcome::Unavailable { reason };
        self.mark_inactive();
        self.rest_raw_mode = false;
    }
}

// ===========================================================================
// Economy-rest raw-copy split (CountryFuel, CountryExperience,
// ProvinceSupply, HubMotorization, DisruptedSupply, ForeignHomebases,
// ResourceDefinitions, CountryResources, ResourceExchanges,
// PurchaseContracts, CountryFuelConsumers).
//
// Eight tables retain their raw mirror and indexed decode. The existing World
// acquisition request also owns native resource-definition, pool and exchange
// rows, acquired by the direct readers with exact per-frame capacity grants.
// All raw staging completes before typed resource acquisition, preventing a
// later raw capacity shortage from retrying an already-read resource source.
//
// Shared raw census spans cover the seven daily raw tables, never ProvinceSupply.
// A source fault there also blocks active typed resource acquisition. When the
// other daily tables are inactive, only ProvinceSupply is staged; country and
// market sources are untouched. Supply-root masks include only active supply tables.
// ===========================================================================

/// Raw bits retain full eleven-table positions; slots6..8 use owned native rows.
const M_E_FUEL: u32 = 0x001;
const M_E_EXP: u32 = 0x002;
const M_E_SUPPLY: u32 = 0x004;
const M_E_HUBS: u32 = 0x008;
const M_E_DISRUPTED: u32 = 0x010;
const M_E_FOREIGN: u32 = 0x020;
const M_E_PURCH: u32 = 0x200;
const M_E_FUEL_CONSUMERS: u32 = 0x400;
/// The fuel status serves both fuel tables; the ring feeds CountryFuel alone.
const M_E_FUEL_STATUS: u32 = M_E_FUEL | M_E_FUEL_CONSUMERS;
const M_E_CENSUS: u32 = 0x63B;
const M_E_SUPPLY_ROOT: u32 = 0x03C;
const M_E_SUPPLY_OBJ: u32 = 0x038;

/// One hour of eight raw tables and three owned native resource tables. Pools
/// recycle without allocation during acquisition; growth runs on the decode
/// side only (see `crate::decode::recycle_economy_rest_job`).
#[derive(Debug, Default)]
pub(crate) struct EconomyRestJob {
    pub log: RawLog<N>,
    pub game_state: usize,
    pub(super) resources_active: bool,
    pub(super) definitions_slot: bool,
    pub(super) other_active: bool,
    pub(super) resources: OwnedResources,
    pub(super) staging_failure: Option<&'static str>,
}

impl EconomyRestJob {
    pub fn new() -> Self {
        Self {
            // Preserve provisioned capacity for the eight remaining raw tables.
            // Resource rows use separate typed storage with exact frame grants.
            log: RawLog::new(1 << 18, 16 << 20),
            game_state: 0,
            resources_active: false,
            definitions_slot: false,
            other_active: false,
            resources: OwnedResources::new([16, 4096, 1024], 1024),
            staging_failure: None,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.resources_active = false;
        self.definitions_slot = false;
        self.other_active = false;
        self.resources.clear();
        self.staging_failure = None;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next() || self.resources.needs_growth()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed walk
    /// in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
        self.resources.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct EconomyRestCopyReport {
    /// False for an unstarted resource group when staging needs capacity and
    /// has not consumed a source fault. Source faults plus overflow are accepted
    /// terminal unavailable payloads, never a reason to reread failed memory.
    pub raw_ok: bool,
    pub copy_ticks: [u64; N],
    /// Indexed raw payload bytes. Owned resource tables have zero raw bytes;
    /// their native row storage is included in raw-memory payload-vector cost.
    pub raw_bytes: [u64; N],
}

/// Acquisition-job parameters supplied at the sampling boundary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EconomyRestCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub resource_rows: [usize; 3],
    pub resource_scratch: usize,
    /// CountryResources, ResourceExchanges and their census are walked
    /// (hourly).
    pub resources_active: bool,
    /// ResourceDefinitions is in its daily slot.
    pub definitions_slot: bool,
    pub other_active: bool,
}

/// Owned native rows only. Capacity grants are the enclosing frame's exact
/// logical limits, not these pools' (potentially larger) physical storage.
#[derive(Debug, Default)]
pub(super) struct OwnedResources {
    pub(super) definitions: Vec<cstrategic_resource::Row>,
    pub(super) resources: Vec<ccountry_resources::Row>,
    pub(super) exchanges: Vec<cresource_exchange::Row>,
    pub(super) scratch: Vec<cresource_exchange::Membership>,
    pub(super) outcomes: [Option<Outcome>; 3],
    pub(super) required: [usize; 3],
    pub(super) scratch_required: usize,
    pub(super) charge: crate::raw_memory::Charge,
}

impl OwnedResources {
    pub(super) fn new(rows: [usize; 3], scratch: usize) -> Self {
        let mut owned = Self {
            required: rows,
            scratch_required: scratch,
            ..Self::default()
        };
        owned.grow();
        owned
    }
    fn clear(&mut self) {
        self.outcomes = [None; 3];
        self.required = [0; 3];
        self.scratch_required = 0;
    }
    fn prepare(&mut self, rows: [usize; 3], scratch: usize) -> bool {
        self.required = rows;
        self.scratch_required = scratch;
        !self.needs_growth()
    }
    pub(super) fn needs_growth(&self) -> bool {
        self.required[0] > self.definitions.len()
            || self.required[1] > self.resources.len()
            || self.required[2] > self.exchanges.len()
            || self.scratch_required > self.scratch.len()
    }
    fn grow(&mut self) {
        grow_vec(&mut self.definitions, self.required[0]);
        grow_vec(&mut self.resources, self.required[1]);
        grow_vec(&mut self.exchanges, self.required[2]);
        grow_vec(&mut self.scratch, self.scratch_required);
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.definitions)
                + crate::raw_memory::vector_bytes(&self.resources)
                + crate::raw_memory::vector_bytes(&self.exchanges)
                + crate::raw_memory::vector_bytes(&self.scratch),
            ..Default::default()
        });
    }
}

impl Captured {
    pub(crate) fn resource_copy_context(
        &mut self,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> EconomyRestCopyContext {
        self.set_schedule(hour, force_full);
        EconomyRestCopyContext {
            base,
            game_state,
            resource_rows: [
                self.definitions.rows.len(),
                self.resources.rows.len(),
                self.exchanges.rows.len(),
            ],
            resource_scratch: self.exchange_scratch.len(),
            resources_active: self.resources_active,
            definitions_slot: self.definitions_slot,
            other_active: self.other_active,
        }
    }
}

pub(super) fn skipped() -> Outcome {
    Outcome::Unavailable {
        reason: replay::cadence::SKIPPED_REASON,
    }
}

fn e_pool_u64(log: &RawLog<N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn e_pool_u32(log: &RawLog<N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn espan(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Bulk variant for contiguous table copies (status blocks, array data).
/// Enclosed sub-reads serve from the bulk span at replay.
fn esbulk(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Mirror of one MSVC string's reads: 32-byte header plus heap payload and
/// terminator. Inline strings ride in the header span.
fn copy_economy_string(log: &mut RawLog<N>, memory: &impl Memory, string: usize, mask: u32) {
    let Some(header_start) = espan(log, memory, Some(string), 32, mask) else {
        return;
    };
    let _ = espan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        mask,
    );
    let _ = espan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        mask,
    );
    let _ = espan(log, memory, Some(string), 8, mask);
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    )) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(s) => s,
            None => return,
        }
    };
    if source.checked_add(len).is_none() {
        return;
    }
    if len != 0 {
        let _ = espan(log, memory, Some(source), len, mask);
    }
    let _ = espan(log, memory, source.checked_add(len), 1, mask);
}

/// Authenticated country census shared by fuel, experience, hubs, disrupted,
/// foreign, resources, exchanges and purchases. Records the exact read set of
/// `ccountry::Reader::new` plus one full `read_at` per slot. Returns the
/// canonical country list (slot, pointer, id) for per-country detail copies.
fn copy_economy_link(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = espan(log, memory, links.data.checked_add(offset), 4, M_E_CENSUS)?;
    let slot = e_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// One structural copy pass over the active raw tables. The hourly supply
/// block is independent of the daily country and market sources.
#[allow(clippy::collapsible_if)]
fn copy_economy_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    other_active: bool,
    log: &mut RawLog<N>,
) {
    let _reads = read_accounting::table(TABLES[2]);
    log.has_bulk = true;
    // Retain the independent hourly sample before a daily census can fail.
    copy_economy_province_supply(log, memory, game_state, other_active);
    if !other_active {
        return;
    }
    // Census runs inline with per-country details copied immediately while
    // iterating slots, parsing link bytes from the log. No heap allocation on
    // the acquisition worker.
    let Some(countries_start) = espan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_E_CENSUS,
    ) else {
        return;
    };
    let Some(links_start) = espan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_E_CENSUS,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(a) => a,
        None => return,
    };
    if countries.count < 2 || links.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    // Market manager shared by purchases.
    let market = game_state
        .checked_add(cgame_state::MARKET_MANAGER)
        .and_then(|p| {
            let s = espan(log, memory, Some(p), 8, M_E_PURCH)?;
            usize::try_from(e_pool_u64(log, s)).ok().filter(|&v| v != 0)
        });
    if let Some(manager) = market {
        let _ = espan(
            log,
            memory,
            manager.checked_add(market_contract_manager::OWNED_CONTRACTS),
            16,
            M_E_PURCH,
        );
        let _ = espan(
            log,
            memory,
            manager.checked_add(market_contract_manager::CONTRACT_TABLE_A),
            16,
            M_E_PURCH,
        );
        let _ = espan(
            log,
            memory,
            manager.checked_add(market_contract_manager::CONTRACT_TABLE_B),
            16,
            M_E_PURCH,
        );
    }
    // Per-country batched gathers: fuel, experience, resources, market status,
    // supply systems with hub/disrupted/foreign details.
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(pointer_start) = espan(log, memory, pointer_addr, 8, M_E_CENSUS) else {
            return;
        };
        let pointer = match usize::try_from(e_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(vtable_start) = espan(log, memory, Some(pointer), 8, M_E_CENSUS) else {
            return;
        };
        let Some(id_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            M_E_CENSUS,
        ) else {
            return;
        };
        if e_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let id = e_pool_u32(log, id_start);
        let canonical = match copy_economy_link(log, memory, &links, countries.count, id) {
            Some(Some(c)) => c,
            _ => return,
        };
        if copy_economy_link(log, memory, &links, countries.count, slot) != Some(Some(canonical)) {
            return;
        }
        if copy_economy_link(log, memory, &links, countries.count, canonical)
            != Some(Some(canonical))
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(canonical_start) = espan(log, memory, canonical_addr, 8, M_E_CENSUS) else {
            return;
        };
        if e_pool_u64(log, canonical_start) != pointer as u64 {
            return;
        }
        if slot != canonical {
            continue;
        }
        // Fuel status block (batched gather): one bulk covers the walk's
        // scattered status reads for both fuel tables, then its consumer
        // entries and history ring.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::FUEL_STATUS),
            8,
            M_E_FUEL_STATUS,
        ) {
            if let Some(status) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                if let Some(start) = esbulk(log, memory, Some(status), 0xe8, M_E_FUEL_STATUS) {
                    copy_fuel_arrays(log, memory, start);
                }
            }
        }
        // Experience block: one contiguous 0x58 bulk.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::EXPERIENCE_STATUS),
            8,
            M_E_EXP,
        ) {
            if let Some(status) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                let _ = esbulk(log, memory, Some(status), 0x58, M_E_EXP);
            }
        }
        // Market status backlink for purchase authentication.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::EQUIPMENT_MARKET_STATUS),
            8,
            M_E_PURCH,
        ) {
            if let Some(status) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                let _ = esbulk(log, memory, Some(status), 0x80, M_E_PURCH);
            }
        }
        if log.overflowed {
            return;
        }
    }
    // Supply per-system details plus market contracts (second loops over the
    // same slots using log-served discovery).
    copy_economy_supply_details(log, memory, base, game_state);
    copy_economy_market_contracts(log, memory, base, market);
}

/// Mirror of the fuel walk's consumer-entry and history-ring reads, found
/// through the headers in the status bulk at `start`. A header the walk
/// refuses stages nothing; the replay refuses it from the same bulk.
fn copy_fuel_arrays(log: &mut RawLog<N>, memory: &impl Memory, start: usize) {
    use ccountry_fuel_status::{CONSUMERS, HISTORY_RING, HISTORY_RING_ENTRIES, MAX_CONSUMERS};
    let header = |log: &RawLog<N>, offset: usize| -> [u8; 16] {
        log.bytes[start + offset..start + offset + 16]
            .try_into()
            .unwrap()
    };
    if let Some(consumers) = Array::from_header(header(log, CONSUMERS), fuel_consumer_data::STRIDE)
        .filter(|array| array.count != 0 && array.count <= MAX_CONSUMERS)
    {
        let bytes = consumers.count as usize * fuel_consumer_data::STRIDE;
        let _ = esbulk(log, memory, Some(consumers.data), bytes, M_E_FUEL_CONSUMERS);
    }
    if let Some(ring) = Array::from_header(header(log, HISTORY_RING), fuel_history_entry::STRIDE)
        .filter(|ring| ring.count == HISTORY_RING_ENTRIES)
    {
        let bytes = HISTORY_RING_ENTRIES as usize * fuel_history_entry::STRIDE;
        let _ = esbulk(log, memory, Some(ring.data), bytes, M_E_FUEL);
    }
}

fn copy_economy_province_supply(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    game_state: usize,
    other_active: bool,
) {
    let root_mask = if other_active {
        M_E_SUPPLY_ROOT
    } else {
        M_E_SUPPLY
    };
    let supply_root = game_state
        .checked_add(cgame_state::SUPPLY_SYSTEM)
        .and_then(|p| {
            let s = espan(log, memory, Some(p), 8, root_mask)?;
            usize::try_from(e_pool_u64(log, s)).ok().filter(|&v| v != 0)
        });
    if let Some(root) = supply_root {
        let _ = esbulk(log, memory, Some(root), 16, root_mask);
        let province_data = espan(
            log,
            memory,
            root.checked_add(engine::csupply_system::PROVINCE_DATA),
            16,
            M_E_SUPPLY,
        );
        if other_active {
            let _ = espan(
                log,
                memory,
                root.checked_add(engine::csupply_system::COUNTRY_SYSTEM_DATA),
                16,
                M_E_SUPPLY_ROOT,
            );
            let _ = espan(
                log,
                memory,
                root.checked_add(engine::csupply_system::COUNTRY_COMPANION_DATA),
                16,
                M_E_SUPPLY_ROOT,
            );
        }
        // Use the first header's recorded bytes to discover its contiguous rows.
        if let Some(start) = province_data {
            let mut header = [0; 16];
            header.copy_from_slice(&log.bytes[start..start + 16]);
            if let Some(array) = Array::from_header(header, sprovince_supply_data::STRIDE)
                && array.count >= 2
            {
                let bytes = (array.count as usize).saturating_mul(sprovince_supply_data::STRIDE);
                let _ = esbulk(log, memory, Some(array.data), bytes, M_E_SUPPLY);
            }
        }
        let _ = espan(
            log,
            memory,
            game_state.checked_add(cgame_state::PROVINCES),
            16,
            M_E_SUPPLY,
        );
    }
}

fn copy_economy_supply_details(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) {
    let _ = base;
    let root = match game_state
        .checked_add(cgame_state::SUPPLY_SYSTEM)
        .and_then(|p| {
            log.spans.iter().rev().find_map(|span| {
                if span.addr == p && span.len == 8 && span.ok {
                    usize::try_from(e_pool_u64(log, span.start as usize)).ok()
                } else {
                    None
                }
            })
        }) {
        Some(r) => r,
        None => return,
    };
    if root == 0 {
        return;
    }
    let systems_header = match root.checked_add(engine::csupply_system::COUNTRY_SYSTEM_DATA) {
        Some(p) => log.spans.iter().rev().find_map(|span| {
            if span.addr == p && span.len == 16 && span.ok {
                let mut h = [0u8; 16];
                h.copy_from_slice(&log.bytes[span.start as usize..span.start as usize + 16]);
                Array::from_header(h, 8)
            } else {
                None
            }
        }),
        None => None,
    };
    let systems = match systems_header {
        Some(a) => a,
        None => return,
    };
    for slot in 1..systems.count {
        let addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| systems.data.checked_add(o));
        let Some(ptr_start) = espan(log, memory, addr, 8, M_E_SUPPLY_OBJ) else {
            continue;
        };
        let system = match usize::try_from(e_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(obj_start) = esbulk(
            log,
            memory,
            Some(system),
            engine::ccountry_supply_system::SIZE,
            M_E_SUPPLY_OBJ,
        ) else {
            continue;
        };
        let mut obj = [0u8; 0x290];
        obj.copy_from_slice(
            &log.bytes[obj_start..obj_start + engine::ccountry_supply_system::SIZE],
        );
        // Hub settings array.
        let hub_data = usize::try_from(u64::from_le_bytes(
            obj[engine::ccountry_supply_system::HUB_SETTINGS_DATA
                ..engine::ccountry_supply_system::HUB_SETTINGS_DATA + 8]
                .try_into()
                .unwrap(),
        ))
        .ok();
        let hub_count = u32::from_le_bytes(
            obj[engine::ccountry_supply_system::HUB_SETTINGS_COUNT
                ..engine::ccountry_supply_system::HUB_SETTINGS_COUNT + 4]
                .try_into()
                .unwrap(),
        );
        if let Some(data) = hub_data.filter(|&d| d != 0 && hub_count != 0) {
            for node_index in 0..hub_count {
                let node_addr = (node_index as usize)
                    .checked_mul(supply_node_record::STRIDE)
                    .and_then(|o| data.checked_add(o));
                let Some(node_start) =
                    esbulk(log, memory, node_addr, supply_node_record::STRIDE, M_E_HUBS)
                else {
                    continue;
                };
                let mut node = [0u8; 0x28];
                node.copy_from_slice(
                    &log.bytes[node_start..node_start + supply_node_record::STRIDE],
                );
                let entries_data = usize::try_from(u64::from_le_bytes(
                    node[supply_node_record::MOTORIZATION_DATA
                        ..supply_node_record::MOTORIZATION_DATA + 8]
                        .try_into()
                        .unwrap(),
                ))
                .ok();
                let entries_count = u32::from_le_bytes(
                    node[supply_node_record::MOTORIZATION_COUNT
                        ..supply_node_record::MOTORIZATION_COUNT + 4]
                        .try_into()
                        .unwrap(),
                );
                if let Some(edata) = entries_data.filter(|&d| d != 0 && entries_count != 0) {
                    let bytes =
                        (entries_count as usize).saturating_mul(hub_motorization_entry::STRIDE);
                    let _ = esbulk(log, memory, Some(edata), bytes, M_E_HUBS);
                }
                if log.overflowed {
                    return;
                }
            }
        }
        // Disrupted buckets: header already in the 0x290 bulk; copy the
        // bucket extent plus end sentinel.
        {
            let buckets = usize::try_from(u64::from_le_bytes(
                obj[engine::ccountry_supply_system::DISRUPTED_BUCKETS
                    ..engine::ccountry_supply_system::DISRUPTED_BUCKETS + 8]
                    .try_into()
                    .unwrap(),
            ))
            .ok()
            .filter(|&p| p != 0);
            let occupied = u32::from_le_bytes(
                obj[engine::ccountry_supply_system::DISRUPTED_COUNT
                    ..engine::ccountry_supply_system::DISRUPTED_COUNT + 4]
                    .try_into()
                    .unwrap(),
            );
            let mask = u32::from_le_bytes(
                obj[engine::ccountry_supply_system::DISRUPTED_MASK
                    ..engine::ccountry_supply_system::DISRUPTED_MASK + 4]
                    .try_into()
                    .unwrap(),
            );
            let max_distance = obj[engine::ccountry_supply_system::DISRUPTED_MAX_DISTANCE];
            if let Some(buckets) = buckets
                && occupied != 0
                && mask.checked_add(1).is_some_and(|c| c.is_power_of_two())
                && max_distance != 0
            {
                let bucket_count = mask + 1;
                if let Some(scan) = bucket_count.checked_add(u32::from(max_distance))
                    && let Some(bytes) = (scan as usize)
                        .checked_mul(engine::disrupted_supply_bucket::STRIDE)
                        .and_then(|b| {
                            b.checked_add(engine::disrupted_supply_bucket::DISTANCE_PLUS_ONE + 1)
                        })
                {
                    let _ = esbulk(log, memory, Some(buckets), bytes, M_E_DISRUPTED);
                }
            }
        }
        // Foreign homebases: array plus records with source strings.
        {
            let foreign_data = usize::try_from(u64::from_le_bytes(
                obj[engine::ccountry_supply_system::FOREIGN_HOMEBASE_DATA
                    ..engine::ccountry_supply_system::FOREIGN_HOMEBASE_DATA + 8]
                    .try_into()
                    .unwrap(),
            ))
            .ok();
            let foreign_count = u32::from_le_bytes(
                obj[engine::ccountry_supply_system::FOREIGN_HOMEBASE_COUNT
                    ..engine::ccountry_supply_system::FOREIGN_HOMEBASE_COUNT + 4]
                    .try_into()
                    .unwrap(),
            );
            if let Some(data) = foreign_data.filter(|&d| d != 0 && foreign_count != 0) {
                for ordinal in 0..foreign_count {
                    let rec_addr = (ordinal as usize)
                        .checked_mul(engine::foreign_homebase_node::STRIDE)
                        .and_then(|o| data.checked_add(o));
                    let Some(rec_start) = esbulk(
                        log,
                        memory,
                        rec_addr,
                        engine::foreign_homebase_node::STRIDE,
                        M_E_FOREIGN,
                    ) else {
                        continue;
                    };
                    // Source string lives at +0x28 inside the record; take
                    // the exact header span plus payload for exact service
                    // (the bulk above already covers the record itself).
                    let _ = rec_start;
                    if let Some(raddr) =
                        rec_addr.and_then(|r| r.checked_add(engine::foreign_homebase_node::SOURCE))
                    {
                        copy_economy_string(log, memory, raddr, M_E_FOREIGN);
                    }
                    if log.overflowed {
                        return;
                    }
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the market contracts: owned vector with 0x260 bulks plus
/// per-country directional vectors. All spans carry the purchases mask.
fn copy_economy_market_contracts(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    base: usize,
    market: Option<usize>,
) {
    let _ = base;
    let manager = match market {
        Some(m) => m,
        None => return,
    };
    let owned = log.spans.iter().rev().find_map(|span| {
        if span.addr == manager + market_contract_manager::OWNED_CONTRACTS
            && span.len == 16
            && span.ok
        {
            let mut h = [0u8; 16];
            h.copy_from_slice(&log.bytes[span.start as usize..span.start as usize + 16]);
            Array::from_header(h, 8)
        } else {
            None
        }
    });
    let owned = match owned {
        Some(a) => a,
        None => return,
    };
    for index in 0..owned.count {
        let addr = (index as usize)
            .checked_mul(8)
            .and_then(|o| owned.data.checked_add(o));
        let Some(ptr_start) = espan(log, memory, addr, 8, M_E_PURCH) else {
            continue;
        };
        let pointer = match usize::try_from(e_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let _ = esbulk(log, memory, Some(pointer), 0x260, M_E_PURCH);
        if log.overflowed {
            return;
        }
    }
    for table in [
        market_contract_manager::CONTRACT_TABLE_A,
        market_contract_manager::CONTRACT_TABLE_B,
    ] {
        let outer = log.spans.iter().rev().find_map(|span| {
            if span.addr == manager + table && span.len == 16 && span.ok {
                let mut h = [0u8; 16];
                h.copy_from_slice(&log.bytes[span.start as usize..span.start as usize + 16]);
                Array::from_header(h, market_contract_manager::COUNTRY_VECTOR_STRIDE)
            } else {
                None
            }
        });
        let outer = match outer {
            Some(a) => a,
            None => continue,
        };
        for slot in 0..outer.count {
            let inner_addr = (slot as usize)
                .checked_mul(market_contract_manager::COUNTRY_VECTOR_STRIDE)
                .and_then(|o| outer.data.checked_add(o));
            let Some(inner_start) = espan(log, memory, inner_addr, 16, M_E_PURCH) else {
                continue;
            };
            let mut h = [0u8; 16];
            h.copy_from_slice(&log.bytes[inner_start..inner_start + 16]);
            if let Some(inner) = Array::from_header(h, 8) {
                for index in 0..inner.count {
                    let addr = (index as usize)
                        .checked_mul(8)
                        .and_then(|o| inner.data.checked_add(o));
                    let _ = espan(log, memory, addr, 8, M_E_PURCH);
                    if log.overflowed {
                        return;
                    }
                }
            }
            if log.overflowed {
                return;
            }
        }
    }
}
