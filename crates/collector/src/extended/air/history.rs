use super::*;

/// Shared regional-history payload slots in report order: [history, damage].
pub(crate) const AIR_HISTORY_N: usize = 2;
const AIR_DAMAGE_MASK: u8 = 2;
const AIR_HISTORY_CENSUS_MASK: u8 = 3;
/// One full acquisition owns native values, not a virtual copy of game memory.
/// The worker copies these values before returning the job to its pool.
#[derive(Debug)]
pub(crate) struct AirHistoryJob {
    pub(super) history: Vec<sair_wing_combat_data::Row>,
    pub(super) damage: Vec<sdamage_entry::Row>,
    pub(super) history_result: Outcome,
    pub(super) damage_result: Outcome,
    pub(super) history_limit: usize,
    pub(super) damage_limit: usize,
    pub(super) required_history: usize,
    pub(super) required_damage: usize,
    pub(super) acquired: bool,
    pub(super) growing: bool,
    pub(super) active: bool,
    pub(super) raw_bytes: [u64; AIR_HISTORY_N],
    pub(super) capacity_charge: crate::raw_memory::Charge,
}
impl Default for AirHistoryJob {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            damage: Vec::new(),
            history_result: Outcome::Unavailable {
                reason: "air history not acquired",
            },
            damage_result: Outcome::Unavailable {
                reason: "air history not acquired",
            },
            history_limit: 0,
            damage_limit: 0,
            required_history: 0,
            required_damage: 0,
            acquired: false,
            growing: false,
            active: false,
            raw_bytes: [0; AIR_HISTORY_N],
            capacity_charge: Default::default(),
        }
    }
}
impl AirHistoryJob {
    pub fn new() -> Self {
        Self::default()
    }
    pub(super) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.history) + vector_bytes(&self.damage),
            ..Capacity::default()
        });
    }
    pub fn has_storage(&self) -> bool {
        !self.history.is_empty() && !self.damage.is_empty()
    }
    pub fn clear(&mut self) {
        self.history_result = Outcome::Unavailable {
            reason: "air history not acquired",
        };
        self.damage_result = self.history_result;
        self.history_limit = 0;
        self.damage_limit = 0;
        self.required_history = 0;
        self.required_damage = 0;
        self.acquired = false;
        self.growing = false;
        self.active = false;
        self.raw_bytes = [0; AIR_HISTORY_N];
    }
    pub fn grow_for_next(&self) -> bool {
        self.required_history > self.history.len() || self.required_damage > self.damage.len()
    }
    pub fn grow(&mut self) {
        fn target(current: usize, required: usize) -> usize {
            if required > current {
                required.max(current.saturating_mul(2))
            } else {
                current
            }
        }
        let history = target(self.history.len(), self.required_history);
        let damage = target(self.damage.len(), self.required_damage);
        crate::table_capture::grow_vec(&mut self.history, history);
        crate::table_capture::grow_vec(&mut self.damage, damage);
        self.update_capacity_charge();
    }
}

/// Meter the existing native walker without retaining a second observation.
struct AirHistoryMemory<'a, M> {
    inner: &'a M,
    raw_bytes: std::cell::Cell<[u64; AIR_HISTORY_N]>,
    record: std::cell::Cell<Option<usize>>,
}
impl<M: Memory> AirHistoryMemory<'_, M> {
    fn structural_mask(&self, address: usize, len: usize) -> u8 {
        if len == 16
            && self.record.get().is_some_and(|record| {
                record.checked_add(sair_wing_combat_data::FIRST_DAMAGE) == Some(address)
                    || record.checked_add(sair_wing_combat_data::SECOND_DAMAGE) == Some(address)
            })
        {
            AIR_DAMAGE_MASK
        } else {
            AIR_HISTORY_CENSUS_MASK
        }
    }
    pub(super) fn count(&self, mask: u8, bytes: usize) {
        let mut values = self.raw_bytes.get();
        let slot = usize::from(mask == AIR_DAMAGE_MASK);
        values[slot] = values[slot].saturating_add(bytes as u64);
        self.raw_bytes.set(values);
    }
}
impl<M: Memory> Memory for AirHistoryMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let bytes = size_of::<T>();
        let value = self.inner.read_scalar::<T>(address);
        if value.is_some() {
            self.count(self.structural_mask(address, bytes), bytes);
        }
        value
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::ReservedCountryObservation> {
        self.inner
            .authenticated_reserved_country_observation(base, state)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let succeeded = self.inner.copy(address, destination);
        if succeeded {
            self.count(
                self.structural_mask(address, destination.len()),
                destination.len(),
            );
        }
        succeeded
    }
    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        let record_leaf = K == 3
            && destinations[0].len() == 26
            && destinations[1].len() == 4
            && destinations[2].len() == 1
            && addresses[0].checked_add(sair_wing_combat_data::ATTRIBUTED_COUNTRY)
                == Some(addresses[1])
            && addresses[0].checked_add(sair_wing_combat_data::COPIED_FLAG) == Some(addresses[2]);
        let damage_leaf = K == 2
            && destinations[0].len() == 12
            && destinations[1].len() == 8
            && addresses[0].checked_add(sdamage_entry::AMOUNT) == Some(addresses[1]);
        if record_leaf {
            self.record.set(Some(addresses[0]));
        }
        let completed = self.inner.copy_scattered(addresses, destinations);
        for (index, destination) in destinations.iter().take(completed).enumerate() {
            let mask = if damage_leaf {
                AIR_DAMAGE_MASK
            } else if record_leaf {
                AIR_HISTORY_CENSUS_MASK
            } else {
                self.structural_mask(addresses[index], destination.len())
            };
            self.count(mask, destination.len());
        }
        completed
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.record.set(None);
        let completed = self.inner.copy_records(sources, layout, destination);
        let count = completed.min(sources.len().saturating_mul(layout.field_count()));
        self.count(AIR_HISTORY_CENSUS_MASK, layout.completed_bytes(count));
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirHistoryCopyReport {
    /// Owned staging ready. Missing row storage uses the existing typed fallback;
    /// a native fault never retries an accepted acquisition.
    pub raw_ok: bool,
    /// Now includes the one direct native walk; worker cost is only installation.
    pub copy_ticks: [u64; AIR_HISTORY_N],
    /// Actual successfully acquired bytes, anchor-charged like the old mirror.
    pub raw_bytes: [u64; AIR_HISTORY_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirHistoryCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    /// Exact native row limits of this frame's destination buffers.
    pub history_limit: usize,
    pub damage_limit: usize,
}

#[cfg(test)]
pub(super) fn acquire_air_history_pass(
    job: &mut AirHistoryJob,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    history_limit: usize,
    damage_limit: usize,
) {
    acquire_air_history_pass_mode(
        job,
        memory,
        base,
        game_state,
        history_limit,
        damage_limit,
        false,
    );
}

fn acquire_air_history_pass_mode(
    job: &mut AirHistoryJob,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    history_limit: usize,
    damage_limit: usize,
    growing: bool,
) {
    let _reads = read_accounting::table(TableId::AirRegionHistory);
    job.clear();
    job.active = true;
    job.growing = growing;
    job.required_history = history_limit;
    job.required_damage = damage_limit;
    job.history_limit = history_limit;
    job.damage_limit = damage_limit;
    if !growing && (history_limit > job.history.len() || damage_limit > job.damage.len()) {
        return;
    }
    let source = AirHistoryMemory {
        inner: memory,
        raw_bytes: std::cell::Cell::new([0; AIR_HISTORY_N]),
        record: std::cell::Cell::new(None),
    };
    let captured = if growing {
        cair_region_combat_data::walk_world_growing(
            &source,
            base,
            game_state,
            &mut job.history,
            &mut job.damage,
        )
    } else {
        cair_region_combat_data::walk_world(
            &source,
            base,
            game_state,
            &mut job.history[..history_limit],
            &mut job.damage[..damage_limit],
        )
    };
    job.history_result = captured.records;
    job.damage_result = captured.damage;
    job.acquired = true;
    job.raw_bytes = source.raw_bytes.get();
}

/// Acquire regional history and damage after checking both destination limits.
pub(crate) fn copy_air_history_refresh_growing(
    memory: &impl Memory,
    ctx: AirHistoryCopyContext,
    job: &mut AirHistoryJob,
) -> AirHistoryCopyReport {
    if !replay::cadence::should_walk(TableId::AirRegionHistory, ctx.hour, ctx.force_full) {
        job.clear();
        return AirHistoryCopyReport::default();
    }
    let started = qpc();
    acquire_air_history_pass_mode(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        ctx.history_limit,
        ctx.damage_limit,
        true,
    );
    AirHistoryCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started), 0],
        raw_bytes: job.raw_bytes,
    }
}

#[cfg(test)]
pub(crate) fn copy_air_history_refresh(
    memory: &impl Memory,
    ctx: AirHistoryCopyContext,
    job: &mut AirHistoryJob,
) -> AirHistoryCopyReport {
    if !replay::cadence::should_walk(TableId::AirRegionHistory, ctx.hour, ctx.force_full) {
        job.clear();
        return AirHistoryCopyReport::default();
    }
    let started = qpc();
    acquire_air_history_pass(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        ctx.history_limit,
        ctx.damage_limit,
    );
    AirHistoryCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started), 0],
        raw_bytes: job.raw_bytes,
    }
}

impl Captured {
    /// Native row limits carried into an independently submitted acquisition.
    pub(crate) fn air_history_limits(&self) -> [usize; AIR_HISTORY_N] {
        [self.history.rows.len(), self.damage.rows.len()]
    }

    /// Install an accepted worker failure after the ordinary capture was gated.
    pub(crate) fn fail_air_history_capture(&mut self, reason: &'static str) {
        self.history.outcome = failed(self.active[3], reason);
        self.damage.outcome = failed(self.active[4], reason);
        self.history_raw_mode = false;
    }

    /// Installation copies only admitted owned row prefixes. Unavailable outcomes
    /// publish no rows, irrespective of any initialized/stale staging tail.
    pub(crate) fn decode_air_history_payload(
        &mut self,
        job: &AirHistoryJob,
        decode_ticks: &mut [u64; AIR_HISTORY_N],
    ) {
        self.active[3] = job.active;
        self.active[4] = job.active;
        if !job.active {
            self.fail_air_history_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let storage = if job.growing {
            engine::walk::Buffer::Growing(&mut self.history.rows)
                .ensure(job.history_result.len())
                .and_then(|_| {
                    engine::walk::Buffer::Growing(&mut self.damage.rows)
                        .ensure(job.damage_result.len())
                })
        } else {
            Ok(true)
        };
        if let Err(reason) = storage {
            self.history.outcome = Outcome::Unavailable { reason };
            self.damage.outcome = self.history.outcome;
        } else if !job.acquired
            || (!job.growing && job.history_limit != self.history.rows.len())
            || (!job.growing && job.damage_limit != self.damage.rows.len())
            || (!job.growing && job.history_limit > job.history.len())
            || (!job.growing && job.damage_limit > job.damage.len())
        {
            self.history.outcome = Outcome::Unavailable {
                reason: "air history owned payload capacity mismatch",
            };
            self.damage.outcome = self.history.outcome;
        } else {
            let history_len = job.history_result.len();
            let damage_len = job.damage_result.len();
            self.history.rows[..history_len].copy_from_slice(&job.history[..history_len]);
            self.damage.rows[..damage_len].copy_from_slice(&job.damage[..damage_len]);
            self.history.outcome = job.history_result;
            self.damage.outcome = job.damage_result;
        }
        decode_ticks[0] += elapsed(started);
        self.history_raw_mode = false;
    }
}
