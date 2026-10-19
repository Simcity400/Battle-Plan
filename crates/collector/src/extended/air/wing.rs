use super::*;
use engine_allocation as allocation;

pub(crate) const WING_PACK_N: usize = 2;
/// Most parts the AirWing census is walked in: the pool runs no more jobs at
/// once.
pub(crate) const WING_PARTS: usize = crate::acquisition::MAX_WORKERS;
/// Wings, plus one per owner with a strategic-air entry, that one census
/// part is sized for.
const PART_COST: u64 = 384;

#[derive(Debug)]
pub(crate) struct AirWingJob {
    pub(crate) crew: crate::crew_store::CrewStore,
    pub(crate) names: Vec<u8>,
    pub(crate) names_len: usize,
    pub(crate) names_required: usize,
    pub(crate) name_failures: usize,
    pub(crate) map_rows: Vec<cair_wing::Row>,
    pub(crate) map_flags: super::wing_map_flags::Store,
    pub(crate) transport: cair_wing::transport_operands::Scratch,
    pub(crate) transport_ticks: u64,
    pub(crate) optional_raw_bytes: u64,
    /// This pass's own sources and memberships. A split census joins only
    /// what each part built from them on its worker: its identities and
    /// naval memberships.
    pub(super) sources: Vec<cair_wing::Source>,
    pub(super) identities: Vec<WingIdentity>,
    pub(super) identity_outcome: Outcome,
    pub(super) equipment: Vec<rows::WingEquipmentRow>,
    pub(super) cumulative: Vec<cair_wing::CumulativeRow>,
    pub(super) memberships: Vec<WingMembershipRead>,
    pub(crate) naval_memberships: Vec<engine_native::ctheatre::WingMembership>,
    pub(crate) naval_membership_outcome: Outcome,
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
    pub(crate) required_memberships: usize,
    pub(super) acquired: bool,
    pub(crate) growing: bool,
    pub(crate) membership_ticks: u64,
    pub(crate) game_score_ticks: [u64; 2],
    pub(super) raw_bytes: [u64; WING_PACK_N],
    pub(super) capacity_charge: crate::raw_memory::Charge,
    /// Held WingEquipment rows ([`super::WingStore`]), lent by the game
    /// thread for one pass and taken back at the join; absent reads every
    /// wing's equipment afresh. A later part of a split census keeps its own.
    pub(crate) wing_store: Option<Box<super::WingStore>>,
    /// What certifies the store's carry: the hooks' domain.
    pub(crate) carry_changes: CarryChanges,
    /// The owner slots this pass walks: every slot, or one part of a census
    /// split by owner range. Set by the callback before each loan.
    pub(crate) slots: std::ops::Range<u32>,
    /// Whether this pass is one part of a split census, which the callback
    /// joins and finishes ([`Self::join_parts`]).
    pub(crate) split: bool,
    /// The census walk as it ended, before the finish.
    pub(crate) census: cair_wing::GameScoreWalk,
    pub(crate) equipment_required: usize,
    pub(crate) cumulative_required: usize,
    pub(crate) membership_len: usize,
    pub(crate) name_failure_reasons: [usize; cair_wing::NameFailure::REASONS.len()],
    /// The WingEquipment carry counts and planned wings of this pass.
    pub(crate) carry: (crate::wing_changes::Counts, usize),
    /// Each owner with a strategic-air entry and the cost (wings and such
    /// owners) walked before it in this pass, and the pass's whole cost:
    /// what the next hour's parts are sized by ([`part_starts`]).
    pub(crate) owner_costs: Vec<(u32, u32)>,
    pub(crate) cost: u32,
}

/// The domain whose certificates carry WingEquipment rows
/// ([`crate::wing_changes`]): the hooks' own, or a test's.
#[derive(Clone, Copy)]
pub(crate) struct CarryChanges(pub(crate) &'static dyn crate::wing_changes::Changes);

impl Default for CarryChanges {
    fn default() -> Self {
        Self(&crate::wing_changes::Observer)
    }
}

impl std::fmt::Debug for CarryChanges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CarryChanges").finish_non_exhaustive()
    }
}

const IDENTITIES_UNALLOCATED: Outcome = Outcome::Unavailable {
    reason: "air wing identity allocation failed",
};

const NO_CENSUS: cair_wing::GameScoreWalk = cair_wing::GameScoreWalk {
    shared: cair_wing::SharedWalk {
        map: NO_WING_SOURCE,
        sources: NO_WING_SOURCE,
        naval: NO_WING_SOURCE,
    },
    score: NO_WING_SOURCE,
    ended: false,
    row_fallbacks: 0,
};

impl Default for AirWingJob {
    fn default() -> Self {
        Self {
            crew: Default::default(),
            names: Vec::new(),
            names_len: 0,
            names_required: 0,
            name_failures: 0,
            map_rows: Vec::new(),
            map_flags: Default::default(),
            transport: Default::default(),
            transport_ticks: 0,
            optional_raw_bytes: 0,
            sources: Vec::new(),
            identities: Vec::new(),
            identity_outcome: NO_WING_SOURCE,
            equipment: Vec::new(),
            cumulative: Vec::new(),
            memberships: Vec::new(),
            naval_memberships: Vec::new(),
            naval_membership_outcome: NO_WING_SOURCE,
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
            growing: false,
            membership_ticks: 0,
            game_score_ticks: [0; 2],
            raw_bytes: [0; 2],
            capacity_charge: Default::default(),
            wing_store: None,
            carry_changes: CarryChanges::default(),
            slots: cair_wing::EVERY_OWNER,
            split: false,
            census: NO_CENSUS,
            equipment_required: 0,
            cumulative_required: 0,
            membership_len: 0,
            name_failure_reasons: [0; cair_wing::NameFailure::REASONS.len()],
            carry: Default::default(),
            owner_costs: Vec::new(),
            cost: 0,
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
            game_score: if enabled {
                crate::extended::game_score::AirInputs::for_wing_job(0)
            } else {
                Default::default()
            },
            game_score_enabled: enabled,
            ..Default::default()
        };
        job.update_capacity_charge();
        job
    }
    /// A later part of a split census: its worker grows its storage, and its
    /// WingEquipment store stays with it.
    pub(crate) fn part() -> Self {
        Self {
            wing_store: cfg!(feature = "wing_changes").then(|| Box::new(super::WingStore::new())),
            ..Default::default()
        }
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
            auxiliary_vectors: vector_bytes(&self.sources)
                + vector_bytes(&self.owner_costs)
                + vector_bytes(&self.identities)
                + vector_bytes(&self.naval_memberships)
                + self.transport.allocated_bytes() as u64
                + self.crew.allocated_bytes() as u64
                + self.map_flags.allocated_bytes(),
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
        self.transport.reset_metadata();
        self.transport_ticks = 0;
        self.optional_raw_bytes = 0;
        self.names_required = 0;
        self.name_failures = 0;
        self.source_len = 0;
        self.map_outcome = NO_WING_SOURCE;
        self.source_outcome = NO_WING_SOURCE;
        self.identity_outcome = NO_WING_SOURCE;
        self.naval_outcome = NO_WING_SOURCE;
        self.naval_membership_outcome = NO_WING_SOURCE;
        self.equipment_outcome = NO_WING_SOURCE;
        self.cumulative_outcome = NO_WING_SOURCE;
        self.cumulative_active = false;
        self.theatres_active = false;
        self.limits = [0; 3];
        self.required = [0; 3];
        self.required_memberships = 0;
        self.acquired = false;
        self.growing = false;
        self.membership_ticks = 0;
        self.game_score_ticks = [0; 2];
        self.raw_bytes = [0; 2];
        self.game_score.begin(false);
        self.census = NO_CENSUS;
        self.equipment_required = 0;
        self.cumulative_required = 0;
        self.membership_len = 0;
        self.name_failure_reasons = [0; cair_wing::NameFailure::REASONS.len()];
        self.carry = Default::default();
        self.owner_costs.clear();
        self.cost = 0;
    }
    pub fn grow_for_next(&self) -> bool {
        self.transport.required_map_capacity > 0
            || !self.transport.fits(self.map_rows.len())
            || self
                .names_required
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
            || self.identity_outcome.len() > self.identities.len()
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
        let _ = self.transport.grow(
            self.map_rows
                .len()
                .max(self.transport.required_map_capacity),
        );
        crate::table_capture::grow_vec(&mut self.equipment, self.required[1]);
        crate::table_capture::grow_vec(&mut self.cumulative, self.required[2]);
        crate::table_capture::grow_vec(&mut self.sources, self.required[0]);
        if self.required_memberships > self.memberships.len() {
            crate::table_capture::grow_vec(&mut self.memberships, self.required_memberships);
        }
        if let Outcome::Truncated { len, unread_rows } = self.source_outcome {
            let required = len
                .saturating_add(unread_rows as usize)
                .max(self.sources.len().saturating_mul(2));
            crate::table_capture::grow_vec(&mut self.sources, required);
        }
        crate::table_capture::grow_vec(&mut self.identities, self.identity_outcome.len());
        if self.game_score_enabled {
            self.game_score.grow_owned();
        }
        self.update_capacity_charge();
    }
    pub(crate) fn memberships(&self) -> (Outcome, &[WingMembershipRead]) {
        (self.naval_outcome, &self.memberships[..self.source_len])
    }
    #[cfg(test)]
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
    optional_bytes: std::cell::Cell<u64>,
}
impl<M: Memory> WingMemory<'_, M> {
    pub(super) fn count(&self, len: usize) {
        let mask = self.mask.get();
        if mask == 0 {
            return;
        }
        if mask == 4 {
            self.optional_bytes
                .set(self.optional_bytes.get().saturating_add(len as u64));
            return;
        }
        let mut total = self.bytes.get();
        let slot = usize::from(mask == 2);
        total[slot] = total[slot].saturating_add(len as u64);
        self.bytes.set(total);
    }
}
impl<M: Memory> Memory for WingMemory<'_, M> {
    fn read_scalar<T: engine_native::memory::Raw>(&self, address: usize) -> Option<T> {
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
    fn copy_records(
        &self,
        sources: engine_native::memory::RecordSources<'_>,
        layout: &engine_native::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.inner.copy_records(sources, layout, destination);
        self.count(layout.completed_bytes(completed));
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirWingCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; WING_PACK_N],
    pub raw_bytes: [u64; WING_PACK_N],
    pub transport_ticks: u64,
    pub game_score_ticks: [u64; 2],
}

impl AirWingCopyReport {
    /// Adds a later census part's worker costs.
    pub(crate) fn add(&mut self, part: &AirWingCopyReport) {
        self.raw_ok &= part.raw_ok;
        for (ours, theirs) in self.copy_ticks.iter_mut().zip(part.copy_ticks) {
            *ours += theirs;
        }
        for (ours, theirs) in self.raw_bytes.iter_mut().zip(part.raw_bytes) {
            *ours += theirs;
        }
        self.transport_ticks += part.transport_ticks;
        for (ours, theirs) in self.game_score_ticks.iter_mut().zip(part.game_score_ticks) {
            *ours += theirs;
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct AirWingCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    /// Initial map grant and independent equipment/cumulative destination limits.
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
        self.statistics.outcome = failed(self.active[4], reason);
        self.stats_raw_mode = false;
    }
}

fn copy_air_pass(
    job: &mut AirWingJob,
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    growing: bool,
) {
    walk_air_part(job, memory, ctx, growing);
    if !job.split {
        job.finish_census(&mut []);
    }
}

/// Walks this job's owner slots of the shared census. A part of a split
/// census grows every projection, so it admits each of its wings as the
/// serial walk would until a projection fails ([`AirWingJob::absorb`]).
fn walk_air_part(
    job: &mut AirWingJob,
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    growing: bool,
) {
    debug_assert!(growing || !job.split, "only growing parts join");
    let mut reads = read_accounting::table(TableId::WingEquipment);
    job.clear();
    job.crew.begin(ctx.limits[0]);
    if growing {
        job.map_flags.begin(ctx.limits[0], ctx.force_full);
    }
    job.growing = growing;
    job.cumulative_active =
        replay_data::cadence::should_walk(TableId::WingCumulative, ctx.hour, ctx.force_full);
    job.theatres_active =
        replay_data::cadence::should_walk(TableId::AirTheatreMembers, ctx.hour, ctx.force_full);
    job.limits = ctx.limits;
    if !job.cumulative_active {
        job.limits[2] = 0;
    }
    job.required = job.limits;
    job.game_score_enabled = ctx.game_score;
    job.game_score.begin(ctx.game_score);
    // The score projection rides this census: a worker pass grows its wing,
    // country and first-site storage from the acquired headers, so ordinary
    // growth never truncates it, whichever frame last lent the storage.
    job.game_score.set_growing(growing);
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
        optional_bytes: std::cell::Cell::new(0),
    };
    let mut equipment = Outcome::Rows { len: 0 };
    let mut equipment_required = 0_usize;
    let mut store = job.wing_store.take();
    let changes = job.carry_changes.0;
    let mut reuse = store.as_deref_mut().map(|store| {
        super::WingReuse::begin(
            store,
            changes,
            crate::sampled_verify::Check::burned_in(ctx.hour, crate::wing_changes::VERIFY),
        )
    });
    let mut cumulative = if job.cumulative_active {
        Outcome::Rows { len: 0 }
    } else {
        SKIPPED
    };
    let mut source_len = 0;
    let mut membership_len = 0;
    let mut membership_ticks = 0;
    let mut cumulative_required = 0;
    let cost = std::cell::Cell::new(0_u32);
    let slots = job.slots.clone();
    let mut transport =
        cair_wing::transport_operands::Pass::new(ctx.base, ctx.limits[0], &mut job.transport);
    let score_ticks = std::cell::Cell::new([0_u64; 2]);
    let score_limit = job.game_score.wing_limit();
    let score = std::cell::RefCell::new(&mut job.game_score);
    let mut map_len = 0;
    let mut fuel_references = engine_native::creference_registry::ScopedResolver::new(ctx.base);
    let map_rows = if growing {
        engine_native::walk::Buffer::Growing(&mut job.map_rows)
    } else {
        engine_native::walk::Buffer::Fixed(&mut job.map_rows[..ctx.limits[0]])
    };
    let sources = if growing {
        engine_native::walk::Buffer::Growing(&mut job.sources)
    } else {
        engine_native::walk::Buffer::Fixed(&mut job.sources)
    };
    let walked = cair_wing::walk_shared_game_score_buffers_with_owners(
        &source,
        ctx.base,
        ctx.game_state,
        slots,
        map_rows,
        sources,
        naval_limit,
        None,
        score_limit,
        |pointer, pool, row, admitted| {
            cost.set(cost.get().saturating_add(1));
            if admitted.map {
                let crew = job.crew.read_air(memory, pointer, row.id);
                row.acquired_manpower = crew.map(|value| value[0]);
                row.exile_country = crew.map(|value| value[1]);
                map_len += 1;
                row.name = if growing {
                    match cair_wing::capture_name_growing(
                        &source,
                        pointer,
                        admitted.name_header,
                        &mut job.names,
                        &mut job.names_len,
                        &mut job.names_required,
                    ) {
                        Ok(span) => Some(span),
                        Err(failure) => {
                            job.name_failures = job.name_failures.saturating_add(1);
                            job.name_failure_reasons[failure.index()] += 1;
                            None
                        }
                    }
                } else {
                    let mut names = cair_wing::NameArena {
                        bytes: &mut job.names,
                        used: job.names_len,
                        required: job.names_required,
                        failures: job.name_failures,
                    };
                    let value = names.capture(&source, pointer, admitted.name_header);
                    job.names_len = names.used;
                    job.names_required = names.required;
                    job.name_failures = names.failures;
                    value
                };
                if growing {
                    let _ = transport.ensure_capacity(map_len);
                    job.map_flags.ensure_capacity(map_len);
                }
                let _transport_reads = read_accounting::table(TableId::AirWings);
                source.mask.set(4);
                let raid_token = growing.then(crate::wing_changes::map_quiet).flatten();
                let started = qpc();
                let raid_pointer = transport.capture(&source, pointer, pool, row);
                job.transport_ticks += elapsed(started);
                source.mask.set(1);
                row.replacement_setting =
                    collector_observers::wing_preferences::read(&source, pointer, row.id);
                if growing {
                    row.fuel_inputs = Some(cair_wing::fuel_inputs::capture(
                        &source,
                        ctx.base,
                        pointer,
                        row,
                        &mut fuel_references,
                    ));
                    row.map_flags = Some(job.map_flags.capture(
                        &source,
                        ctx.base,
                        pointer,
                        row,
                        raid_pointer,
                        raid_token,
                    ));
                    if row.map_flags.is_some_and(|flags| flags & 16 != 0) {
                        row.raid_reference = job
                            .map_flags
                            .raid_reference(pointer, row.id)
                            .filter(|reference| reference.resolves)
                            .or(row.raid_reference);
                    }
                }
            }
            let wing = WingSource { pointer, pool, row };
            if admitted.sources && !matches!(equipment, Outcome::Unavailable { .. }) {
                source.mask.set(1);
                let output = if growing {
                    engine_native::walk::Buffer::Growing(&mut job.equipment)
                } else {
                    engine_native::walk::Buffer::Fixed(&mut job.equipment[..ctx.limits[1]])
                };
                let (next, required) = read_equipment_carried(
                    &source,
                    ctx.base,
                    &wing,
                    output,
                    equipment.len(),
                    reuse.as_mut(),
                );
                equipment_required = equipment_required.saturating_add(required);
                equipment = append(equipment, next);
            }
            if admitted.sources && !matches!(cumulative, Outcome::Unavailable { .. }) {
                reads.select(TableId::WingCumulative);
                source.mask.set(2);
                let mut output = if growing {
                    engine_native::walk::Buffer::Growing(&mut job.cumulative)
                } else {
                    engine_native::walk::Buffer::Fixed(&mut job.cumulative[..ctx.limits[2]])
                };
                let required = cumulative.len().saturating_add(1);
                cumulative_required = required;
                let next = if let Err(reason) = output.ensure(required) {
                    Outcome::Unavailable { reason }
                } else if let Some(destination) = output.get_mut(cumulative.len()) {
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
                if growing
                    && engine_native::walk::Buffer::Growing(&mut job.memberships)
                        .ensure(membership_len + 1)
                        .is_err()
                {
                    return;
                }
                let _reads = read_accounting::table(TableId::AirTheatreMembers);
                source.mask.set(0);
                let started = qpc();
                job.memberships[membership_len] =
                    match engine_native::ctheatre::read_wing_membership(&source, pointer, row) {
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
            if entry.is_some() {
                // Advisory: a missed owner only sizes the next hour's parts.
                if allocation::advisory(|| allocation::reserve(&mut job.owner_costs, 1)).is_ok() {
                    job.owner_costs.push((slot, cost.get()));
                }
                cost.set(cost.get().saturating_add(1));
            }
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
    job.crew.finish();
    if growing {
        job.map_flags.finish();
    }
    job.census = walked;
    job.equipment_outcome = equipment;
    job.cumulative_outcome = cumulative;
    job.equipment_required = equipment_required;
    job.cumulative_required = cumulative_required;
    if let Some(reuse) = reuse {
        reuse.settle();
    }
    job.carry = store
        .as_deref()
        .map(super::WingStore::tally)
        .unwrap_or_default();
    job.wing_store = store;
    job.source_len = source_len;
    job.membership_len = membership_len;
    job.membership_ticks = membership_ticks;
    job.raw_bytes = source.bytes.get();
    job.optional_raw_bytes = source.optional_bytes.get();
    job.acquired = true;
    job.game_score_ticks = score_ticks.get();
    job.cost = cost.get();
    if growing {
        job.project();
    }
}

impl AirWingJob {
    /// The callback's join of a split census: `parts`, the census parts of
    /// the owner slots after this one's in slot order, then the finish.
    /// Copies without allocating once this job holds the storage of the
    /// last joined census (its frame grants and grown vectors).
    pub(crate) fn join_parts(&mut self, parts: &mut [AirWingJob]) -> Result<(), &'static str> {
        let joined = parts.iter().try_for_each(|part| self.absorb(part));
        if joined.is_ok() {
            self.finish_census(parts);
        }
        // The join's growth, or a failed join's, is this job's storage.
        self.update_capacity_charge();
        joined
    }

    /// This part's own mission identities and naval memberships, built from
    /// its sources and memberships on its worker; a split census joins them.
    fn project(&mut self) {
        let sources = self.census.shared.sources;
        let len = sources.len();
        self.identity_outcome = if crate::table_capture::grow_vec(&mut self.identities, len) {
            let mut outcome = sources;
            if matches!(outcome, Outcome::Rows { .. }) {
                for (destination, source) in
                    self.identities[..len].iter_mut().zip(&self.sources[..len])
                {
                    match WingIdentity::new(source.pointer, source.pool, &source.row) {
                        Ok(identity) => *destination = identity,
                        Err(reason) => {
                            outcome = Outcome::Unavailable { reason };
                            break;
                        }
                    }
                }
            }
            outcome
        } else {
            IDENTITIES_UNALLOCATED
        };
        if !self.theatres_active {
            self.naval_membership_outcome = SKIPPED;
            return;
        }
        let len = self.source_len;
        self.naval_membership_outcome =
            if crate::table_capture::grow_vec(&mut self.naval_memberships, len) {
                let mut outcome = self.census.shared.naval;
                for (destination, original) in self.naval_memberships[..len]
                    .iter_mut()
                    .zip(&self.memberships[..len])
                {
                    *destination = original.value;
                    if let Some(reason) = original.error {
                        outcome = Outcome::Unavailable { reason };
                        break;
                    }
                }
                outcome
            } else {
                Outcome::Unavailable {
                    reason: "air wing naval membership allocation failed",
                }
            };
    }

    /// Joins `part` as one walk of both would have left the census. The
    /// serial walk stops where a part ended, so nothing after it joins. In a
    /// growing pass the map, source and naval projections admit every wing
    /// until they fail together, after which the serial walk reads only
    /// game-score leaves and owners: a later part's census rows, names,
    /// identities, equipment, cumulative rows, naval memberships and carry
    /// counts then join no outcome and stay out, as do those of a dependent
    /// table that failed. Each part's own first identity or membership
    /// refusal leads its projection, so the first in slot order is the
    /// census's.
    fn absorb(&mut self, part: &AirWingJob) -> Result<(), &'static str> {
        if self.census.ended {
            return Ok(());
        }
        if !matches!(self.census.shared.map, Outcome::Unavailable { .. }) {
            self.absorb_census(part)?;
        }
        if self.game_score_enabled {
            self.game_score
                .absorb(&part.game_score, part.slots.start as usize)?;
        }
        let (ours, theirs) = (self.census, part.census);
        self.census = cair_wing::GameScoreWalk {
            shared: cair_wing::SharedWalk {
                map: append(ours.shared.map, theirs.shared.map),
                sources: append(ours.shared.sources, theirs.shared.sources),
                naval: append(ours.shared.naval, theirs.shared.naval),
            },
            score: append(ours.score, theirs.score),
            ended: theirs.ended,
            row_fallbacks: ours.row_fallbacks.saturating_add(theirs.row_fallbacks),
        };
        Ok(())
    }

    fn absorb_census(&mut self, part: &AirWingJob) -> Result<(), &'static str> {
        let names = self.names_len;
        join(
            &mut self.names,
            names,
            &part.names[..part.names_len],
            |&byte| byte,
        )?;
        self.names_len += part.names_len;
        self.names_required = self.names_required.saturating_add(part.names_required);
        self.name_failures = self.name_failures.saturating_add(part.name_failures);
        for (ours, theirs) in self
            .name_failure_reasons
            .iter_mut()
            .zip(part.name_failure_reasons)
        {
            *ours += theirs;
        }
        self.transport.failures += part.transport.failures;
        self.optional_raw_bytes += part.optional_raw_bytes;
        let rebase = |row: &cair_wing::Row| cair_wing::Row {
            name: row.name.map(|(start, len)| (start + names, len)),
            ..*row
        };
        join(
            &mut self.map_rows,
            self.census.shared.map.len(),
            &part.map_rows[..part.census.shared.map.len()],
            rebase,
        )?;
        if !matches!(self.identity_outcome, Outcome::Unavailable { .. }) {
            join(
                &mut self.identities,
                self.identity_outcome.len(),
                &part.identities[..part.identity_outcome.len()],
                |&identity| identity,
            )?;
            self.identity_outcome = append(self.identity_outcome, part.identity_outcome);
        }
        if !matches!(self.equipment_outcome, Outcome::Unavailable { .. }) {
            join(
                &mut self.equipment,
                self.equipment_outcome.len(),
                &part.equipment[..part.equipment_outcome.len()],
                Clone::clone,
            )?;
            self.equipment_outcome = append(self.equipment_outcome, part.equipment_outcome);
            self.equipment_required = self
                .equipment_required
                .saturating_add(part.equipment_required);
            self.carry.0.add(&part.carry.0);
            self.carry.1 += part.carry.1;
        }
        if self.cumulative_active && !matches!(self.cumulative_outcome, Outcome::Unavailable { .. })
        {
            let len = self.cumulative_outcome.len();
            join(
                &mut self.cumulative,
                len,
                &part.cumulative[..part.cumulative_outcome.len()],
                |&row| row,
            )?;
            if part.cumulative_required != 0 {
                self.cumulative_required = len + part.cumulative_required;
            }
            self.cumulative_outcome = append(self.cumulative_outcome, part.cumulative_outcome);
        }
        if self.theatres_active {
            // Each part's memberships stay with it; their demand is the
            // census's.
            self.membership_len += part.membership_len;
        }
        if self.theatres_active
            && !matches!(self.naval_membership_outcome, Outcome::Unavailable { .. })
        {
            let len = self.naval_membership_outcome.len();
            // Shorter than its outcome only after a membership storage
            // refusal, which the consumer truncates to the outcome too.
            let joined = part
                .naval_membership_outcome
                .len()
                .min(part.naval_memberships.len());
            join(
                &mut self.naval_memberships,
                len,
                &part.naval_memberships[..joined],
                |&membership| membership,
            )?;
            self.naval_membership_outcome =
                append(self.naval_membership_outcome, part.naval_membership_outcome);
        }
        Ok(())
    }

    /// Completes the census from its walk: the game-score, AirWings, source,
    /// naval and dependent outcomes and every part's WingEquipment store.
    fn finish_census(&mut self, parts: &mut [AirWingJob]) {
        crate::map_capture::observe_air_wing_name_failures(&self.name_failure_reasons);
        crate::map_capture::observe_air_wing_name_refusals(self.name_failures);
        let walked = self.census;
        self.game_score.finish(walked.score);
        self.map_outcome = walked.shared.map;
        self.source_outcome = walked.shared.sources;
        self.naval_outcome = if self.theatres_active {
            walked.shared.naval
        } else {
            SKIPPED
        };
        self.equipment_outcome = require_parent(walked.shared.sources, self.equipment_outcome);
        let complete = matches!(self.equipment_outcome, Outcome::Rows { .. });
        let stores = std::iter::once(&mut self.wing_store)
            .chain(parts.iter_mut().map(|part| &mut part.wing_store))
            .filter_map(|store| store.as_deref_mut());
        let mut carried = false;
        for store in stores {
            store.conclude(complete);
            carried = true;
        }
        if carried {
            let (counts, wings) = self.carry;
            crate::wing_changes::record(&counts, complete.then_some(wings));
        }
        self.cumulative_outcome = if self.cumulative_active {
            require_parent(walked.shared.sources, self.cumulative_outcome)
        } else {
            SKIPPED
        };
        self.required[1] = self.required[1].max(self.equipment_required);
        self.required[2] = self.required[2].max(self.cumulative_required);
        if self.growing {
            self.required[0] = self.required[0].max(self.map_outcome.len());
            self.required_memberships = self.required_memberships.max(self.membership_len);
            // Identities exist only for a complete source census: a later
            // part's census refusal is the joined identities' outcome.
            if !matches!(self.source_outcome, Outcome::Rows { .. })
                && self.identity_outcome != IDENTITIES_UNALLOCATED
            {
                self.identity_outcome = self.source_outcome;
            }
        }
    }
}

/// The callback's account of the WingEquipment stores after the join: the
/// first job's and every part's, idle ones included, each forgotten when the
/// acquisition `failed`. Returns the bytes they retain.
pub(crate) fn retained_store_bytes(
    census: Option<&mut super::WingStore>,
    parts: &mut [AirWingJob],
    failed: bool,
) -> u64 {
    let stores = census.into_iter().chain(
        parts
            .iter_mut()
            .filter_map(|part| part.wing_store.as_deref_mut()),
    );
    let mut bytes = 0;
    for store in stores {
        if failed {
            store.forget();
        }
        bytes += store.allocated_bytes() as u64;
    }
    bytes
}

#[cfg(test)]
std::thread_local! {
    /// Test seam: how many more of the join's growths succeed before one is
    /// refused, as a refused allocation is.
    static JOIN_GROWTHS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Copies `rows`, converted, after the first `len` of `destination`.
fn join<T: Default, U>(
    destination: &mut Vec<T>,
    len: usize,
    rows: &[U],
    convert: impl Fn(&U) -> T,
) -> Result<(), &'static str> {
    let end = len
        .checked_add(rows.len())
        .ok_or("air wing census join overflow")?;
    #[cfg(test)]
    if destination.len() < end
        && JOIN_GROWTHS.with(|left| {
            let refused = left.get() == Some(0);
            left.set(left.get().and_then(|left| left.checked_sub(1)));
            refused
        })
    {
        return Err("capture buffer allocation failed");
    }
    Buffer::Growing(destination).ensure(end)?;
    for (destination, row) in destination[len..end].iter_mut().zip(rows) {
        *destination = convert(row);
    }
    Ok(())
}

/// Where the next hour's census parts start, after the first: one part per
/// [`PART_COST`] of the wings and owners this hour's joined `census` and its
/// `parts` walked, each from the owner holding its share's first unit. None
/// when the census did not visit every wing.
pub(crate) fn part_starts(
    census: &AirWingJob,
    parts: &[AirWingJob],
) -> Option<([u32; WING_PARTS - 1], usize)> {
    if !matches!(census.map_outcome, Outcome::Rows { .. }) {
        return None;
    }
    let jobs = || std::iter::once(census).chain(parts);
    let total: u64 = jobs().map(|job| u64::from(job.cost)).sum();
    let count = total.div_ceil(PART_COST).clamp(1, WING_PARTS as u64);
    let mut starts = [0; WING_PARTS - 1];
    let mut filled = 0;
    let mut start = |slot: u32| {
        if slot > 0 && (filled == 0 || slot > starts[filled - 1]) {
            starts[filled] = slot;
            filled += 1;
        }
    };
    let (mut part, mut offset, mut holder) = (1, 0, None);
    for job in jobs() {
        for &(slot, before) in &job.owner_costs {
            // The owner before this one holds every unit up to here.
            while part < count && offset + u64::from(before) > total * part / count {
                if let Some(slot) = holder {
                    start(slot);
                }
                part += 1;
            }
            holder = Some(slot);
        }
        offset += u64::from(job.cost);
    }
    for _ in part..count {
        if let Some(slot) = holder {
            start(slot);
        }
    }
    Some((starts, filled))
}

#[cfg(test)]
pub(crate) fn copy_air_refresh(
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    job: &mut AirWingJob,
) -> AirWingCopyReport {
    copy_air_refresh_inner(memory, ctx, job, false)
}

pub(crate) fn copy_air_refresh_growing(
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    job: &mut AirWingJob,
) -> AirWingCopyReport {
    copy_air_refresh_inner(memory, ctx, job, true)
}

fn copy_air_refresh_inner(
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    job: &mut AirWingJob,
    growing: bool,
) -> AirWingCopyReport {
    let started = qpc();
    let cumulative_active =
        replay_data::cadence::should_walk(TableId::WingCumulative, ctx.hour, ctx.force_full);
    let theatres_active =
        replay_data::cadence::should_walk(TableId::AirTheatreMembers, ctx.hour, ctx.force_full);
    let mut limits = ctx.limits;
    if !cumulative_active {
        limits[2] = 0;
    }
    let memberships = if theatres_active {
        ctx.limits[0].max(ctx.naval_limit)
    } else {
        0
    };
    if growing {
        job.required = limits;
        job.required_memberships = memberships;
        if job.grow_for_next() {
            job.grow();
        }
    }
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
    if !growing && !fits {
        job.clear();
        job.cumulative_active = cumulative_active;
        job.theatres_active = theatres_active;
        job.required = limits;
        job.required_memberships = memberships;
        return AirWingCopyReport::default();
    }
    copy_air_pass(job, memory, ctx, growing);
    if growing {
        job.update_capacity_charge();
    }
    AirWingCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [
            elapsed(started)
                .saturating_sub(job.membership_ticks)
                .saturating_sub(job.transport_ticks)
                .saturating_sub(job.game_score_ticks.iter().sum()),
            0,
        ],
        raw_bytes: job.raw_bytes,
        transport_ticks: job.transport_ticks,
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
    if job.growing {
        crate::table_capture::grow_vec(&mut captured.equipment.rows, job.equipment_outcome.len());
        if job.cumulative_active {
            crate::table_capture::grow_vec(
                &mut captured.cumulative.rows,
                job.cumulative_outcome.len(),
            );
        }
    }
    let fits = if job.growing {
        captured.equipment.rows.len() >= job.equipment_outcome.len()
            && (!job.cumulative_active
                || captured.cumulative.rows.len() >= job.cumulative_outcome.len())
    } else {
        captured.equipment.rows.len() == job.limits[1]
            && (!job.cumulative_active || captured.cumulative.rows.len() == job.limits[2])
    };
    if !job.acquired || !fits {
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

#[cfg(test)]
mod split_tests;
#[cfg(test)]
mod transport_tests;
