use super::*;
use engine::allocation;

pub(crate) const WING_PACK_N: usize = 2;

#[derive(Debug)]
pub(crate) struct AirWingJob {
    pub(crate) names: Vec<u8>,
    pub(crate) names_len: usize,
    pub(crate) names_required: usize,
    pub(crate) name_failures: usize,
    pub(crate) map_rows: Vec<cair_wing::Row>,
    pub(super) sources: Vec<cair_wing::Source>,
    pub(super) equipment: Vec<rows::WingEquipmentRow>,
    pub(super) cumulative: Vec<cair_wing::CumulativeRow>,
    pub(super) memberships: Vec<WingMembershipRead>,
    pub(crate) game_score: crate::extended::game_score::AirInputs,
    pub(super) game_score_enabled: bool,
    pub(super) source_len: usize,
    pub(crate) map_outcome: Outcome,
    pub(crate) source_outcome: Outcome,
    pub(super) naval_outcome: Outcome,
    pub(super) equipment_outcome: Outcome,
    pub(super) cumulative_outcome: Outcome,
    pub(super) cumulative_active: bool,
    pub(crate) theatres_active: bool,
    pub(super) limits: [usize; 3],
    pub(super) required: [usize; 3],
    pub(super) required_memberships: usize,
    pub(super) acquired: bool,
    pub(crate) membership_ticks: u64,
    pub(crate) game_score_ticks: [u64; 2],
    pub(super) raw_bytes: [u64; WING_PACK_N],
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

impl Default for AirWingJob {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            names_len: 0,
            names_required: 0,
            name_failures: 0,
            map_rows: Vec::new(),
            sources: Vec::new(),
            equipment: Vec::new(),
            cumulative: Vec::new(),
            memberships: Vec::new(),
            game_score: Default::default(),
            game_score_enabled: false,
            source_len: 0,
            map_outcome: NO_WING_SOURCE,
            source_outcome: NO_WING_SOURCE,
            naval_outcome: NO_WING_SOURCE,
            equipment_outcome: NO_WING_SOURCE,
            cumulative_outcome: NO_WING_SOURCE,
            cumulative_active: false,
            theatres_active: false,
            limits: [0; 3],
            required: [0; 3],
            required_memberships: 0,
            acquired: false,
            membership_ticks: 0,
            game_score_ticks: [0; 2],
            raw_bytes: [0; 2],
            capacity_charge: Default::default(),
        }
    }
}

impl AirWingJob {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_game_score(false)
    }
    pub(crate) fn with_game_score(enabled: bool) -> Self {
        let mut job = Self {
            names: allocation::owned_vec(allocation::default_vec(
                crate::country_scale::air_wing_name_bytes(),
            )),
            map_rows: allocation::owned_vec(allocation::default_vec(8192)),
            sources: allocation::owned_vec(allocation::default_vec(8192)),
            equipment: allocation::owned_vec(allocation::default_vec(8192)),
            cumulative: allocation::owned_vec(allocation::default_vec(4096)),
            memberships: allocation::owned_vec(allocation::default_vec(8192)),
            game_score: if enabled {
                crate::extended::game_score::AirInputs::for_wing_job(8192)
            } else {
                Default::default()
            },
            game_score_enabled: enabled,
            ..Default::default()
        };
        job.update_capacity_charge();
        job
    }
    pub(crate) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.map_rows)
                + vector_bytes(&self.equipment)
                + vector_bytes(&self.cumulative)
                + vector_bytes(&self.memberships)
                + vector_bytes(&self.names)
                + self.game_score.allocated_bytes() as u64,
            auxiliary_vectors: vector_bytes(&self.sources),
            ..Default::default()
        });
    }
    pub(crate) fn presize_game_score(&mut self) {
        if !self.game_score_enabled {
            return;
        }
        self.game_score.presize();
        self.update_capacity_charge();
    }
    pub fn has_storage(&self) -> bool {
        !self.sources.is_empty()
    }
    pub fn clear(&mut self) {
        self.names_len = 0;
        self.names_required = 0;
        self.name_failures = 0;
        self.source_len = 0;
        self.map_outcome = NO_WING_SOURCE;
        self.source_outcome = NO_WING_SOURCE;
        self.naval_outcome = NO_WING_SOURCE;
        self.equipment_outcome = NO_WING_SOURCE;
        self.cumulative_outcome = NO_WING_SOURCE;
        self.cumulative_active = false;
        self.theatres_active = false;
        self.limits = [0; 3];
        self.required = [0; 3];
        self.required_memberships = 0;
        self.acquired = false;
        self.membership_ticks = 0;
        self.game_score_ticks = [0; 2];
        self.raw_bytes = [0; 2];
        self.game_score.begin(false);
    }
    pub fn grow_for_next(&self) -> bool {
        self.names_required
            .max(crate::country_scale::air_wing_name_bytes())
            > self.names.len()
            || self
                .required
                .iter()
                .zip([
                    self.map_rows.len(),
                    self.equipment.len(),
                    self.cumulative.len(),
                ])
                .any(|(&required, present)| required > present)
            || matches!(self.source_outcome, Outcome::Truncated { .. })
            || self.required_memberships > self.memberships.len()
            || (self.game_score_enabled && self.game_score.grow_for_next())
    }
    pub fn grow(&mut self) {
        crate::table_capture::grow_vec(
            &mut self.names,
            self.names_required
                .max(crate::country_scale::air_wing_name_bytes()),
        );
        crate::table_capture::grow_vec(&mut self.map_rows, self.required[0]);
        crate::table_capture::grow_vec(&mut self.equipment, self.required[1]);
        crate::table_capture::grow_vec(&mut self.cumulative, self.required[2]);
        if self.sources.is_empty() {
            crate::table_capture::grow_vec(&mut self.sources, 8192);
        }
        if self.required_memberships > self.memberships.len() {
            crate::table_capture::grow_vec(&mut self.memberships, self.required_memberships);
        }
        if let Outcome::Truncated { len, unread_rows } = self.source_outcome {
            let required = len
                .saturating_add(unread_rows as usize)
                .max(self.sources.len().saturating_mul(2));
            crate::table_capture::grow_vec(&mut self.sources, required);
        }
        if self.game_score_enabled {
            self.game_score.grow_owned();
        }
        self.update_capacity_charge();
    }
    pub(crate) fn memberships(&self) -> (Outcome, &[WingMembershipRead]) {
        (self.naval_outcome, &self.memberships[..self.source_len])
    }
    pub(crate) fn map_memberships(&self) -> &[WingMembershipRead] {
        if self.theatres_active {
            &self.memberships[..self.map_outcome.len()]
        } else {
            &[]
        }
    }
}

/// Successful native byte counts for the shared wing census and its extensions.
struct WingMemory<'a, M> {
    inner: &'a M,
    bytes: std::cell::Cell<[u64; 2]>,
    mask: std::cell::Cell<u8>,
}
impl<M: Memory> WingMemory<'_, M> {
    pub(super) fn count(&self, len: usize) {
        let mask = self.mask.get();
        if mask == 0 {
            return;
        }
        let mut total = self.bytes.get();
        let slot = usize::from(mask == 2);
        total[slot] = total[slot].saturating_add(len as u64);
        self.bytes.set(total);
    }
}
impl<M: Memory> Memory for WingMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.inner.read_scalar::<T>(address);
        if value.is_some() {
            self.count(size_of::<T>());
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
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ok = self.inner.copy(address, destination);
        if ok {
            self.count(destination.len());
        }
        ok
    }
    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        let completed = self.inner.copy_scattered(addresses, destinations);
        for destination in destinations.iter().take(completed) {
            self.count(destination.len());
        }
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirWingCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; WING_PACK_N],
    pub raw_bytes: [u64; WING_PACK_N],
    pub game_score_ticks: [u64; 2],
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct AirWingCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    /// Independent map, equipment and cumulative destination limits.
    pub limits: [usize; 3],
    /// Independently pooled naval source capacity.
    pub naval_limit: usize,
    pub game_score: bool,
}

impl Captured {
    pub(crate) fn wing_limits(&self) -> [usize; 2] {
        [self.equipment.rows.len(), self.cumulative.rows.len()]
    }

    /// Complete a failed owned wing job after the typed visitors were gated.
    /// Other air tables retain their independently acquired results and costs.
    pub(crate) fn fail_air_wing_capture(&mut self, reason: &'static str) {
        self.equipment.outcome = Outcome::Unavailable { reason };
        self.cumulative.outcome = failed(self.active[2], reason);
    }

    /// Complete a failed statistics job after its typed walk was gated.
    pub(crate) fn fail_air_stats_capture(&mut self, reason: &'static str) {
        self.statistics.outcome = failed(self.active[6], reason);
        self.stats_raw_mode = false;
    }
}

fn copy_air_pass(job: &mut AirWingJob, memory: &impl Memory, ctx: AirWingCopyContext) {
    let mut reads = read_accounting::table(TableId::WingEquipment);
    job.clear();
    job.cumulative_active =
        replay::cadence::should_walk(TableId::WingCumulative, ctx.hour, ctx.force_full);
    job.theatres_active =
        replay::cadence::should_walk(TableId::AirTheatreMembers, ctx.hour, ctx.force_full);
    job.limits = ctx.limits;
    if !job.cumulative_active {
        job.limits[2] = 0;
    }
    job.required = job.limits;
    job.game_score_enabled = ctx.game_score;
    job.game_score.begin(ctx.game_score);
    let naval_limit = if job.theatres_active {
        ctx.naval_limit
    } else {
        0
    };
    job.required_memberships = if job.theatres_active {
        ctx.limits[0].max(naval_limit)
    } else {
        0
    };
    let source = WingMemory {
        inner: memory,
        bytes: std::cell::Cell::new([0; 2]),
        mask: std::cell::Cell::new(3),
    };
    let mut equipment = Outcome::Rows { len: 0 };
    let mut cumulative = if job.cumulative_active {
        Outcome::Rows { len: 0 }
    } else {
        SKIPPED
    };
    let mut source_len = 0;
    let mut membership_len = 0;
    let mut membership_ticks = 0;
    let mut names = cair_wing::NameArena {
        bytes: &mut job.names,
        used: 0,
        required: 0,
        failures: 0,
    };
    let score_ticks = std::cell::Cell::new([0_u64; 2]);
    let score_limit = job.game_score.wing_limit();
    let score = std::cell::RefCell::new(&mut job.game_score);
    let walked = cair_wing::walk_shared_game_score_with_owners(
        &source,
        ctx.base,
        ctx.game_state,
        &mut job.map_rows[..ctx.limits[0]],
        &mut job.sources,
        naval_limit,
        Some(&mut names),
        score_limit,
        |pointer, pool, row, admitted| {
            let wing = WingSource { pointer, pool, row };
            if admitted.sources && !matches!(equipment, Outcome::Unavailable { .. }) {
                source.mask.set(1);
                let next = read_equipment(
                    &source,
                    ctx.base,
                    &wing,
                    &mut job.equipment[equipment.len()..ctx.limits[1]],
                );
                equipment = append(equipment, next);
            }
            if admitted.sources && !matches!(cumulative, Outcome::Unavailable { .. }) {
                reads.select(TableId::WingCumulative);
                source.mask.set(2);
                let next = if let Some(destination) =
                    job.cumulative[..ctx.limits[2]].get_mut(cumulative.len())
                {
                    match cair_wing::read_cumulative(&source, pointer, row) {
                        Ok(value) => {
                            *destination = value;
                            Outcome::Rows { len: 1 }
                        }
                        Err(reason) => Outcome::Unavailable { reason },
                    }
                } else {
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 1,
                    }
                };
                cumulative = append(cumulative, next);
            }
            if job.theatres_active && (admitted.map || admitted.naval) {
                let _reads = read_accounting::table(TableId::AirTheatreMembers);
                source.mask.set(0);
                let started = qpc();
                job.memberships[membership_len] =
                    match engine::ctheatre::read_wing_membership(&source, pointer, row) {
                        Ok(value) => WingMembershipRead { value, error: None },
                        Err(reason) => WingMembershipRead {
                            error: Some(reason),
                            ..Default::default()
                        },
                    };
                membership_ticks += elapsed(started);
                membership_len += 1;
                source_len += usize::from(admitted.naval);
            }
            source.mask.set(3);
            reads.select(TableId::WingEquipment);
        },
        |pointer, row| {
            let _reads = read_accounting::table(TableId::AirWingGameScoreStats);
            source.mask.set(0);
            let started = qpc();
            score.borrow_mut().wing(&source, pointer, row);
            let mut ticks = score_ticks.get();
            ticks[1] = ticks[1].saturating_add(elapsed(started));
            score_ticks.set(ticks);
            source.mask.set(3);
        },
        |slot, entry| {
            if !ctx.game_score {
                return;
            }
            let _reads = read_accounting::table(TableId::CountryGameScoreInputs);
            source.mask.set(0);
            let started = qpc();
            score.borrow_mut().owner(&source, ctx.base, slot, entry);
            let mut ticks = score_ticks.get();
            ticks[0] = ticks[0].saturating_add(elapsed(started));
            score_ticks.set(ticks);
            source.mask.set(3);
        },
    );
    job.names_len = names.used;
    job.names_required = names.required;
    job.name_failures = names.failures;
    crate::map_capture::observe_air_wing_name_refusals(names.failures);
    score.borrow_mut().finish(walked.score);
    job.map_outcome = walked.shared.map;
    job.source_outcome = walked.shared.sources;
    job.naval_outcome = if job.theatres_active {
        walked.shared.naval
    } else {
        SKIPPED
    };
    job.equipment_outcome = require_parent(walked.shared.sources, equipment);
    job.cumulative_outcome = if job.cumulative_active {
        require_parent(walked.shared.sources, cumulative)
    } else {
        SKIPPED
    };
    job.source_len = source_len;
    job.membership_ticks = membership_ticks;
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
    job.game_score_ticks = score_ticks.get();
}

pub(crate) fn copy_air_refresh(
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    job: &mut AirWingJob,
) -> AirWingCopyReport {
    let started = qpc();
    let cumulative_active =
        replay::cadence::should_walk(TableId::WingCumulative, ctx.hour, ctx.force_full);
    let theatres_active =
        replay::cadence::should_walk(TableId::AirTheatreMembers, ctx.hour, ctx.force_full);
    let mut limits = ctx.limits;
    if !cumulative_active {
        limits[2] = 0;
    }
    let memberships = if theatres_active {
        ctx.limits[0].max(ctx.naval_limit)
    } else {
        0
    };
    let fits = job.has_storage()
        && job.memberships.len() >= memberships
        && limits
            .iter()
            .zip([
                job.map_rows.len(),
                job.equipment.len(),
                job.cumulative.len(),
            ])
            .all(|(&required, capacity)| required <= capacity);
    if !fits {
        job.clear();
        job.cumulative_active = cumulative_active;
        job.theatres_active = theatres_active;
        job.required = limits;
        job.required_memberships = memberships;
        return AirWingCopyReport::default();
    }
    copy_air_pass(job, memory, ctx);
    AirWingCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [
            elapsed(started)
                .saturating_sub(job.membership_ticks)
                .saturating_sub(job.game_score_ticks.iter().sum()),
            0,
        ],
        raw_bytes: job.raw_bytes,
        game_score_ticks: job.game_score_ticks,
    }
}

pub(crate) fn decode_air_payload(
    captured: &mut Captured,
    job: &AirWingJob,
    decode_ticks: &mut [u64; WING_PACK_N],
) {
    let started = qpc();
    captured.active[2] = job.cumulative_active;
    if !job.acquired
        || captured.equipment.rows.len() != job.limits[1]
        || (job.cumulative_active && captured.cumulative.rows.len() != job.limits[2])
    {
        captured.equipment.outcome = NO_WING_SOURCE;
        captured.cumulative.outcome = if job.cumulative_active {
            NO_WING_SOURCE
        } else {
            SKIPPED
        };
    } else {
        let equipment = job.equipment_outcome.len();
        let cumulative = job.cumulative_outcome.len();
        // WingEquipmentRow owns only integer scalars; cloning does not allocate.
        captured.equipment.rows[..equipment].clone_from_slice(&job.equipment[..equipment]);
        if job.cumulative_active {
            captured.cumulative.rows[..cumulative].copy_from_slice(&job.cumulative[..cumulative]);
        }
        captured.equipment.outcome = job.equipment_outcome;
        captured.cumulative.outcome = job.cumulative_outcome;
    }
    decode_ticks[0] += elapsed(started);
}
