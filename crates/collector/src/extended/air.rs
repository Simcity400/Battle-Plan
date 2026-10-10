//! Pooled air state. Allocating acquisition, conversion and growth are worker-only.
//!
//! A single independently bounded wing census acquires map, equipment,
//! cumulative and naval-membership value rows. The worker installs owned
//! extension rows without replaying native memory reads. The same census,
//! joined by each wing's mission address, orders the registered missions and
//! names the bases and mines the flying-aircraft map needs.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, table_iter, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, allocation, cace, cair_base, cair_region_combat_data,
    cair_regional_mission::{self, WingIdentity},
    cair_wing, cair_wing_pool, ccountry, ccountry_air_container, cgame_state, cloop_history,
    cpdx_array::Array,
    cprovince, cstate, cstrategic_air_manager, cstrategic_navy_manager, equipment_entry,
    map_air_base_reference,
    memory::{RecordLayout, RecordSources},
    msvc_string, sair_wing_combat_data, sdamage_entry,
    walk::{Buffer, Outcome},
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

use super::{
    air_histories,
    air_reference_witness::{self, BaseWitness},
    air_sorties::{self, Sorties},
};

#[path = "transport_air.rs"]
mod transport_air;

pub(crate) fn transport_status() -> impl serde::Serialize {
    transport_air::status()
}

pub(crate) struct MissionSources<'a, C> {
    pub parent: Outcome,
    pub histories: &'a mut [air_histories::Staging],
    pub changes: &'a C,
    pub transport_regions: Result<&'a [rows::TransportSpottingRegion], &'static str>,
    pub transport_guard: Result<(), &'static str>,
    pub transport_admission: Result<(), &'static str>,
}

pub const N: usize = 17;
/// The tables `Capacities::rows` sizes; the sortie tables size themselves.
pub const SIZED: usize = 11;
pub const TABLES: [TableId; N] = [
    TableId::WingEquipment,
    TableId::AirBases,
    TableId::WingCumulative,
    TableId::AirRegionHistory,
    TableId::AirRegionDamage,
    TableId::AirAces,
    TableId::AirStatistics,
    TableId::AirRegionalMissions,
    TableId::AirReferenceWitnesses,
    TableId::NavalMineLevels,
    TableId::AirMissionInputs,
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
    TableId::AirSortieModifierMetadata,
    TableId::AirNavalPatrolCombats,
    TableId::RaidAirScores,
];

/// The regional source walk is charged to history. Owned wing acquisition is charged to
/// WingEquipment, excluding separately charged naval-membership reads. Typed
/// fallback extension times exclude the enclosing map census.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("regional air history and damage", 3, &[3, 4])];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    /// History/damage capacities count native entries, before off-thread grouping.
    /// Witnesses count probed base handles.
    pub rows: [usize; SIZED],
    pub ace_names: usize,
    /// The wing census joined to missions, and its base-handle scratch.
    pub wings: usize,
    pub sorties: air_sorties::Capacities,
    /// RaidAirScores rows, region roster and strategic-air countries.
    pub scores: ScoreCapacities,
    /// Physical roots, regional headers, mission occurrences, factors and entries.
    pub(crate) transport: [usize; 5],
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ScoreCapacities {
    pub rows: usize,
    pub regions: usize,
    pub countries: usize,
}

impl Capacities {
    /// Off-thread attach/cadence hints; capture still follows its own original
    /// headers and grows every worker arena before publishing the hour.
    pub(crate) fn presize_missions(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let regions = state
            .checked_add(cgame_state::STRATEGIC_REGIONS)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("air mission sizing region roster unreadable")?;
        self.scores.regions = self.scores.regions.max(regions.count as usize);
        let mut countries = 0usize;
        let mut missions = 0usize;
        // RegisterMission@140C4E360 appends the same ordinary occurrences to
        // +B0 and +98; +C8 additionally includes carrier-only regions.
        engine::cstrategic_air::visit(memory, base, state, |root| {
            let active = root
                .address
                .checked_add(engine::cstrategic_air::ALL_ACTIVE_MISSIONS)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("air mission sizing active vector unreadable")?;
            missions = missions
                .checked_add(active.count as usize)
                .ok_or("air mission sizing count overflow")?;
            countries = countries
                .checked_add(1)
                .ok_or("air mission sizing country count overflow")?;
            Ok(())
        })?;
        self.rows[7] = self.rows[7].max(missions);
        self.rows[9] = self.rows[9].max(missions);
        self.scores.countries = self.scores.countries.max(countries);
        self.scores.rows = self.scores.rows.max(countries);
        let wings = crate::country_scale::table_demand::rows(cair_wing::walk(
            memory,
            base,
            state,
            &mut [],
        ))?;
        let handles = wings
            .checked_mul(2)
            .ok_or("air reference sizing count overflow")?;
        self.wings = self.wings.max(wings).max(missions);
        self.rows[8] = self.rows[8].max(handles);
        // 141237D20 reaches native task-force combat occurrences. Every
        // task-force roster occurrence is an independent upper bound.
        let task_forces = engine::cfleet::walk(memory, base, state, &mut [], &mut [], &mut []);
        self.rows[10] = self.rows[10].max(crate::country_scale::table_demand::rows(
            task_forces.task_forces,
        )?);
        Ok(())
    }

    pub(crate) fn raise_for_transport(&mut self, demand: [usize; 5]) {
        for (capacity, count) in self.transport.iter_mut().zip(demand) {
            *capacity = (*capacity).max(count);
        }
    }

    pub(crate) fn raise_for_names(&mut self, demand: crate::country_scale::AceNamesDemand) {
        self.rows[5] = self.rows[5].max(demand.aces);
        self.ace_names = self.ace_names.max(demand.ace_bytes);
    }

    /// Every strategic-air country holds a RaidAirScores witness row; its
    /// nonzero pairs grow the table on demand.
    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.scores.countries, slots);
        crate::country_scale::raise(&mut self.scores.rows, slots);
    }
}

/// The exact invocation context from the shared authenticated AirWings visitor.
/// The wing and pool pointers go into this hour's `WingIdentity` census,
/// which lives only for the acquisition: the sortie capture reads each pool's
/// definition map flags through the pool pointer. No pointer reaches the
/// replay.
pub struct WingSource<'a> {
    pub pointer: usize,
    pub pool: usize,
    pub row: &'a cair_wing::Row,
}

pub struct Captured {
    transport: transport_air::Captured,
    equipment: CaptureRows<rows::WingEquipmentRow>,
    bases: CaptureRows<cair_base::Row>,
    carrier: carrier::Bases,
    cumulative: CaptureRows<cair_wing::CumulativeRow>,
    history: CaptureRows<sair_wing_combat_data::Row>,
    damage: CaptureRows<sdamage_entry::Row>,
    aces: CaptureRows<cace::Row>,
    statistics: CaptureRows<cloop_history::Row>,
    scores: CaptureRows<engine::raid_air_scores::Row>,
    /// Held-table sizes: this pack's capacities, then the last walk's demand.
    score_capacity: [usize; 2],
    score_demand: [usize; 2],
    names: Arena<u8>,
    /// This hour's complete wing census by mission address. Acquisition
    /// context only: pointers never reach the replay.
    identities: Arena<WingIdentity>,
    identity_outcome: Outcome,
    /// The owned wing job installed the census; the typed visitor did not run.
    identities_owned: bool,
    registrations: CaptureRows<cair_regional_mission::Row>,
    registration_failure: cair_regional_mission::FailureContext,
    /// Base handles to probe: registered pool bases, unregistered training
    /// wings' pool bases and every transfer base.
    handles: Vec<u64>,
    references: CaptureRows<BaseWitness>,
    sorties: Sorties,
    mines: CaptureRows<rows::NavalMineLevelRow>,
    patrol_combats: CaptureRows<rows::AirNavalPatrolCombatRow>,
    /// The registrations' and combats' ticks once the AirMissions job, or
    /// the Late job without it, captured them this hour.
    missions: Option<[u64; 2]>,
    wing_ticks: [u64; 2],
    /// Game-thread time installing the owned census, charged to
    /// AirRegionalMissions, which joins it.
    identity_ticks: u64,
    active: [bool; N],
    /// Raw-copy split gate for the scheduled air-statistics table. Set by the
    /// callback on hours with a valid raw payload; `capture` then skips
    /// the typed statistics walk and the decode worker fills these rows
    /// before the ready handoff. Preserved across `reset` (set per hour,
    /// cleared by the worker after decode).
    pub(crate) stats_raw_mode: bool,
    /// Raw-copy split gate for the shared bases/registrations/aces payload
    /// (copy/decode split). Set by the game thread every hour with a valid
    /// raw payload; `capture` then skips the typed bases and aces walks and
    /// the decode worker fills those rows (plus the fog registrations) before
    /// the ready handoff. Preserved across `reset`, cleared by the worker.
    pub(crate) bases_raw_mode: bool,
    /// Raw-copy split gate for the shared history/damage payload
    /// (copy/decode split). Set by the callback on scheduled hours with a valid
    /// raw payload; `capture` then skips the typed regional walk and the
    /// decode worker fills both row sets before the ready handoff. Preserved
    /// across `reset`, cleared by the worker.
    pub(crate) history_raw_mode: bool,
}

const NO_WING_SOURCE: Outcome = Outcome::Unavailable {
    reason: "shared air wing census not supplied",
};

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay::cadence::SKIPPED_REASON,
};

fn failed(active: bool, reason: &'static str) -> Outcome {
    if active {
        Outcome::Unavailable { reason }
    } else {
        SKIPPED
    }
}

impl Captured {
    /// An accepted Late job never retries. Independent owned payloads still
    /// install their tables on decode; their raw gates must survive this path,
    /// and so do the AirMissions job's registrations and combats.
    pub(crate) fn fail_late_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        self.scores.fail(failed(self.active[16], reason));
        self.equipment.fail(failed(self.active[0], reason));
        self.cumulative.fail(failed(self.active[2], reason));
        if self.missions.is_none() {
            self.fail_missions_rows(reason);
        }
        self.references.fail(failed(self.active[8], reason));
        self.sorties.fail(
            failed(self.active[10], reason),
            failed(self.active[14], reason),
        );
        self.mines.fail(failed(self.active[9], reason));
        if !self.bases_raw_mode {
            self.bases.fail(failed(self.active[1], reason));
            self.aces.fail(failed(self.active[5], reason));
        }
        if !self.history_raw_mode {
            self.history.fail(failed(self.active[3], reason));
            self.damage.fail(failed(self.active[4], reason));
        }
        if !self.stats_raw_mode {
            self.statistics.fail(failed(self.active[6], reason));
        }
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; SIZED],
            ace_names: 0,
            wings: 0,
            sorties: air_sorties::Capacities {
                missions: 0,
                reached: 0,
                rows: 0,
                keys: 0,
                hosts: 0,
            },
            scores: ScoreCapacities {
                rows: 0,
                regions: 0,
                countries: 0,
            },
            transport: [0; 5],
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        let mut captured = Self {
            transport: transport_air::Captured::new(
                capacity.scores.countries.max(capacity.transport[0]),
                capacity.rows[7].max(capacity.transport[1]),
                capacity.wings.max(capacity.transport[2]),
                capacity.scores.rows.max(capacity.transport[3]),
            ),
            equipment: CaptureRows::new(capacity.rows[0]),
            bases: CaptureRows::new(capacity.rows[1]),
            carrier: carrier::Bases::new(capacity.rows[1]),
            cumulative: CaptureRows::new(capacity.rows[2]),
            history: CaptureRows::new(capacity.rows[3]),
            damage: CaptureRows::new(capacity.rows[4]),
            aces: CaptureRows::new(capacity.rows[5]),
            statistics: CaptureRows::new(capacity.rows[6]),
            scores: CaptureRows::new(capacity.scores.rows),
            score_capacity: [capacity.scores.regions, capacity.scores.countries],
            score_demand: [0; 2],
            names: Arena::new(capacity.ace_names),
            identities: Arena::new(capacity.wings),
            identity_outcome: NO_WING_SOURCE,
            identities_owned: false,
            registrations: CaptureRows::new(capacity.rows[7]),
            registration_failure: Default::default(),
            handles: allocation::owned_vec(allocation::filled(capacity.wings.saturating_mul(2), 0)),
            references: CaptureRows::new(capacity.rows[8]),
            sorties: Sorties::with_capacities(&capacity.sorties),
            mines: CaptureRows::new(capacity.rows[9]),
            patrol_combats: CaptureRows::new(capacity.rows[10]),
            missions: None,
            wing_ticks: [0; 2],
            identity_ticks: 0,
            active: [true; N],
            stats_raw_mode: false,
            bases_raw_mode: false,
            history_raw_mode: false,
        };
        captured
            .transport
            .prepare_normalization(capacity.transport[4]);
        captured
    }

    /// Reset before the shared map walk. `capture` deliberately preserves the
    /// wing extension state prepared by begin/visit/finish_wings.
    pub fn reset(&mut self) {
        self.equipment.reset();
        self.equipment.outcome = NO_WING_SOURCE;
        self.bases.reset();
        self.carrier.reset();
        self.cumulative.reset();
        self.cumulative.outcome = NO_WING_SOURCE;
        self.history.reset();
        self.damage.reset();
        self.aces.reset();
        self.statistics.reset();
        self.scores.reset();
        self.score_demand = [0; 2];
        self.names.reset();
        self.identities.reset();
        self.identity_outcome = NO_WING_SOURCE;
        self.identities_owned = false;
        self.reset_missions();
        self.wing_ticks = [0; 2];
        self.identity_ticks = 0;
    }

    fn reset_missions(&mut self) {
        self.transport.reset();
        self.registrations.reset();
        self.registration_failure = Default::default();
        self.patrol_combats.reset();
        self.missions = None;
        self.reset_joins();
    }

    /// The Late job's joins of the registered missions.
    fn reset_joins(&mut self) {
        self.references.reset();
        self.sorties.reset();
        self.mines.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.transport.allocated_bytes()
            + self.equipment.allocated_bytes()
            + self.bases.allocated_bytes()
            + self.carrier.bytes()
            + self.cumulative.allocated_bytes()
            + self.history.allocated_bytes()
            + self.damage.allocated_bytes()
            + self.aces.allocated_bytes()
            + self.statistics.allocated_bytes()
            + self.scores.allocated_bytes()
            + self.names.allocated_bytes()
            + self.identities.allocated_bytes()
            + self.registrations.allocated_bytes()
            + self.handles.capacity() * size_of::<u64>()
            + self.references.allocated_bytes()
            + self.sorties.allocated_bytes()
            + self.mines.allocated_bytes()
            + self.patrol_combats.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.equipment.grow(&mut capacity.rows[0]);
        self.bases.grow(&mut capacity.rows[1]);
        crate::table_capture::grow_vec(&mut self.carrier.rows, capacity.rows[1]);
        self.carrier.grow();
        self.cumulative.grow(&mut capacity.rows[2]);
        self.history.grow(&mut capacity.rows[3]);
        self.damage.grow(&mut capacity.rows[4]);
        self.aces.grow(&mut capacity.rows[5]);
        self.statistics.grow(&mut capacity.rows[6]);
        self.scores.grow(&mut capacity.scores.rows);
        let scores = &mut capacity.scores;
        crate::table_capture::grow_capacity(&mut scores.regions, self.score_demand[0]);
        crate::table_capture::grow_capacity(&mut scores.countries, self.score_demand[1]);
        self.score_capacity = [scores.regions, scores.countries];
        self.names.grow(&mut capacity.ace_names);
        self.identities.grow(&mut capacity.wings);
        // One pool base per registration plus one transfer base per wing.
        crate::table_capture::grow_vec(
            &mut self.handles,
            capacity.wings.saturating_mul(2).max(capacity.rows[8]),
        );
        self.registrations.grow(&mut capacity.rows[7]);
        self.references.grow(&mut capacity.rows[8]);
        self.sorties.grow(&mut capacity.sorties);
        self.mines.grow(&mut capacity.rows[9]);
        self.patrol_combats.grow(&mut capacity.rows[10]);
        self.transport.prepare(
            capacity.scores.countries.max(capacity.transport[0]),
            capacity.rows[7].max(capacity.transport[1]),
            capacity.wings.max(capacity.transport[2]),
            capacity.scores.rows.max(capacity.transport[3]),
        );
        self.transport.prepare_normalization(capacity.transport[4]);
    }

    /// Freeze the requested tables before contexts and shared map callbacks.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.transport.set_hour(hour);
        self.active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
    }

    /// Whether this hour's registrations and combats were captured.
    pub(crate) fn missions_captured(&self) -> bool {
        self.missions.is_some()
    }

    /// The AirMissions job, before the Late job, which joins what it
    /// leaves: the registered missions, then the naval-patrol combats, whose
    /// regions and every registered mission region open the hour's history
    /// parts ([`air_histories::supply`]).
    /// `parent` is the map census the typed visitor supplied the wing
    /// identities in, when it did.
    pub(crate) fn capture_missions(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        sources: MissionSources<'_, impl crate::history_changes::Changes>,
    ) {
        let MissionSources {
            parent,
            histories,
            changes,
            transport_regions,
            transport_guard,
            transport_admission,
        } = sources;
        if !self.identities_owned {
            self.identity_outcome = require_parent(parent, self.identity_outcome);
        }
        self.registrations.reset();
        self.registration_failure = Default::default();
        self.patrol_combats.reset();
        let begin = qpc();
        self.registrations.outcome = if !self.active[7] {
            SKIPPED
        } else if let Outcome::Rows { len } = self.identity_outcome {
            let _reads = read_accounting::table(TableId::AirRegionalMissions);
            let identities = &mut self.identities.first[..len];
            identities.sort_unstable_by_key(|wing| wing.mission_pointer);
            let original = if transport_admission.is_ok() {
                let mut observer = self.transport.observer_growing(
                    memory,
                    base,
                    game_state,
                    transport_regions,
                    transport_guard,
                );
                cair_regional_mission::walk_with_transport_growing(
                    memory,
                    base,
                    game_state,
                    identities,
                    &mut self.registrations.rows,
                    &mut self.registration_failure,
                    &mut observer,
                )
            } else {
                // Preserve the original strict product without acquiring any
                // transport root/factor/wing/normalization words. Its already
                // copied numerical census still establishes capacity demand.
                let outcome = cair_regional_mission::walk_with_diagnostic_growing(
                    memory,
                    base,
                    game_state,
                    identities,
                    &mut self.registrations.rows,
                    &mut self.registration_failure,
                );
                self.transport.denied(len, outcome);
                outcome
            };
            bounded(original, self.registrations.rows.len())
        } else {
            Outcome::Unavailable {
                reason: "registered missions lack the complete wing census",
            }
        };
        self.transport.finish(self.registrations.outcome);
        let registered_ticks = elapsed(begin).saturating_add(self.identity_ticks);
        let begin = qpc();
        let (wings, registered) = joins(
            &mut self.identities,
            self.identity_outcome,
            &self.registrations,
        );
        // Every hour: the histories, which need the regions, are sliced.
        let regions = {
            let _reads = read_accounting::table(TableId::AirNavalPatrolCombats);
            capture_patrol_combats(
                memory,
                base,
                game_state,
                wings,
                registered,
                Buffer::Growing(&mut self.handles),
                Buffer::Growing(&mut self.patrol_combats.rows),
            )
        };
        self.patrol_combats.outcome = match regions {
            _ if !self.active[15] => SKIPPED,
            Ok((outcome, _)) => outcome,
            Err(reason) => Outcome::Unavailable { reason },
        };
        let regions = scenario_history_regions_buffer(
            wings,
            registered,
            regions,
            Buffer::Growing(&mut self.handles),
        );
        air_histories::supply(
            histories,
            regions.map(|count| &self.handles[..count]),
            changes,
            &mut air_histories::opening(),
        );
        self.missions = Some([registered_ticks, elapsed(begin)]);
    }

    /// The AirMissions job was interrupted: publish its tables unavailable;
    /// the history parts have no regions. Nothing is read again.
    pub(crate) fn fail_missions(
        &mut self,
        reason: &'static str,
        histories: &mut [air_histories::Staging],
    ) {
        self.transport.fail();
        self.fail_missions_rows(reason);
        self.missions = Some([0; 2]);
        air_histories::supply(
            histories,
            Err(reason),
            &crate::history_changes::Unobserved,
            &mut air_histories::opening(),
        );
    }

    pub(crate) fn transport_air_packet(
        &self,
        hour: i32,
    ) -> std::io::Result<Option<rows::TransportAirInputs>> {
        self.transport.packet(hour)
    }

    /// A failed original numerical-region loan association withdraws only
    /// the optional transport graph; the strict air tables keep their result.
    pub(crate) fn refuse_transport_regions(&mut self) {
        self.transport.fail();
    }

    fn fail_missions_rows(&mut self, reason: &'static str) {
        self.registration_failure = Default::default();
        self.registrations.fail(failed(self.active[7], reason));
        self.patrol_combats.fail(failed(self.active[15], reason));
    }

    /// Initialize the extension rows immediately before the shared map walk.
    pub fn begin_wings(&mut self) {
        self.equipment.reset();
        self.cumulative.reset();
        self.equipment.outcome = Outcome::Rows { len: 0 };
        self.cumulative.outcome = if self.active[2] {
            Outcome::Rows { len: 0 }
        } else {
            SKIPPED
        };
        self.identities.reset();
        self.identity_outcome = Outcome::Rows { len: 0 };
        self.identities_owned = false;
        self.wing_ticks = [0; 2];
        self.identity_ticks = 0;
    }

    fn push_identity(&mut self, pointer: usize, pool: usize, row: &cair_wing::Row) {
        if matches!(self.identity_outcome, Outcome::Unavailable { .. }) {
            return;
        }
        let identity = match WingIdentity::new(pointer, pool, row) {
            Ok(identity) => identity,
            Err(reason) => {
                self.identity_outcome = Outcome::Unavailable { reason };
                return;
            }
        };
        self.identities.required += 1;
        if let Some(destination) = self.identities.first.get_mut(self.identities.len) {
            *destination = identity;
            self.identities.len += 1;
        }
        let len = self.identities.len;
        let unread_rows = (self.identities.required - len).min(u32::MAX as usize) as u32;
        self.identity_outcome = if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        };
    }

    /// The owned wing job's complete source census, installed on the game
    /// thread after the job joins; the typed visitor does not run that hour.
    pub(crate) fn install_owned_mission_identities(&mut self, job: &mut AirWingJob) {
        let begin = qpc();
        if job.identity_outcome != NO_WING_SOURCE {
            let len = job.identity_outcome.len();
            if len > self.identities.first.len() {
                std::mem::swap(&mut self.identities.first, &mut job.identities);
            } else {
                self.identities.first[..len].copy_from_slice(&job.identities[..len]);
            }
            self.identities.len = len;
            self.identities.required = len;
            self.identity_outcome = job.identity_outcome;
            self.identities_owned = true;
        } else {
            self.install_identities(job);
        }
        self.identity_ticks = self.identity_ticks.saturating_add(elapsed(begin));
    }

    fn install_identities(&mut self, job: &AirWingJob) {
        self.identities.reset();
        self.identities_owned = true;
        self.identity_outcome = Outcome::Rows { len: 0 };
        match job.source_outcome {
            Outcome::Rows { len } if len <= job.sources.len() => {
                for source in &job.sources[..len] {
                    self.push_identity(source.pointer, source.pool, &source.row);
                }
            }
            _ => {
                self.identity_outcome = Outcome::Unavailable {
                    reason: "registered missions lack the complete wing census",
                }
            }
        }
    }

    /// Extension failure never changes the map row. Equipment retains full
    /// native references and amounts, with no transient pointers in pooled rows.
    pub fn visit_wing(&mut self, memory: &impl Memory, base: usize, source: WingSource<'_>) {
        self.push_identity(source.pointer, source.pool, source.row);
        let mut reads = read_accounting::table(TableId::WingEquipment);
        let begin = qpc();
        let outcome = &mut self.equipment.outcome;
        if !matches!(outcome, Outcome::Unavailable { .. }) {
            let next = read_equipment(
                memory,
                base,
                &source,
                &mut self.equipment.rows[outcome.len()..],
            );
            *outcome = append(*outcome, next);
        }
        let end = qpc();
        self.wing_ticks[0] =
            self.wing_ticks[0].saturating_add(end.saturating_sub(begin).max(0) as u64);
        if !self.active[2] {
            self.cumulative.outcome = SKIPPED;
            return;
        }
        reads.select(TableId::WingCumulative);
        let outcome = &mut self.cumulative.outcome;
        if !matches!(outcome, Outcome::Unavailable { .. }) {
            let next = if let Some(destination) = self.cumulative.rows.get_mut(outcome.len()) {
                match cair_wing::read_cumulative(memory, source.pointer, source.row) {
                    Ok(row) => {
                        *destination = row;
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
            *outcome = append(*outcome, next);
        }
        self.wing_ticks[1] = self.wing_ticks[1].saturating_add(elapsed(end));
    }

    /// Both extensions require a complete parent census. Subtract these ticks
    /// from the enclosing map walk's charged time.
    pub fn finish_wings(&mut self, source: Outcome) -> [u64; 2] {
        if !self.identities_owned {
            self.identity_outcome = require_parent(source, self.identity_outcome);
        }
        self.equipment.outcome = require_parent(source, self.equipment.outcome);
        self.cumulative.outcome = if self.active[2] {
            require_parent(source, self.cumulative.outcome)
        } else {
            SKIPPED
        };
        self.wing_ticks
    }

    /// Monotonic nested equipment/cumulative ticks for the map adapter. Reading
    /// this before/after each visitor or finish call avoids charging work twice.
    pub fn wing_ticks(&self) -> [u64; 2] {
        self.wing_ticks
    }

    /// The independent sources and the joins of the registered missions, after
    /// the extensions supplied by the map visitor. Reset non-wing buffers here,
    /// preserving the prepared extensions. `scores` is the attachment's held
    /// RaidAirScores table, loaned each hour.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        scores: &mut engine::raid_air_scores::Held,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.bases.reset();
        self.carrier.reset();
        self.history.reset();
        self.damage.reset();
        self.aces.reset();
        self.statistics.reset();
        self.scores.reset();
        self.score_demand = [0; 2];
        self.names.reset();
        self.reset_joins();
        let mut ticks = [0; N];
        ticks[0] = self.wing_ticks[0];
        ticks[2] = self.wing_ticks[1];
        if !self.bases_raw_mode {
            let begin = qpc();
            capture_rows(&mut self.bases, |rows| {
                let _reads = read_accounting::table(TableId::AirBases);
                cair_base::walk(memory, base, game_state, rows)
            });
            ticks[1] = elapsed(begin);
        }
        if !self.active[3] {
            self.history.outcome = SKIPPED;
            self.damage.outcome = SKIPPED;
        } else if !self.history_raw_mode {
            let _reads = read_accounting::table(TableId::AirRegionHistory);
            let begin = qpc();
            let first = cair_region_combat_data::walk_world(
                memory,
                base,
                game_state,
                &mut self.history.rows,
                &mut self.damage.rows,
            );
            self.history.outcome = bounded(first.records, self.history.rows.len());
            self.damage.outcome = bounded(first.damage, self.damage.rows.len());
            ticks[3] = elapsed(begin);
        }
        if !self.active[5] {
            self.aces.outcome = SKIPPED;
        } else if !self.bases_raw_mode {
            let _reads = read_accounting::table(TableId::AirAces);
            let begin = qpc();
            let first = cace::walk(
                memory,
                base,
                game_state,
                &mut self.aces.rows,
                &mut self.names.first,
            );
            self.aces.outcome = names_outcome(first, &self.aces.rows, &self.names.first);
            self.names.len = first.bytes_len;
            self.names.required = first.required_bytes;
            ticks[5] = elapsed(begin);
        }
        if !self.active[6] {
            self.statistics.outcome = SKIPPED;
        } else if !self.stats_raw_mode {
            let begin = qpc();
            capture_rows(&mut self.statistics, |rows| {
                let _reads = read_accounting::table(TableId::AirStatistics);
                cloop_history::walk(memory, base, game_state, rows)
            });
            ticks[6] = elapsed(begin);
        }
        [ticks[7], ticks[15]] = self.missions.unwrap_or_default();
        let (wings, registered) = joins(
            &mut self.identities,
            self.identity_outcome,
            &self.registrations,
        );
        let begin = qpc();
        self.references.outcome = if !self.active[8] {
            SKIPPED
        } else {
            let _reads = read_accounting::table(TableId::AirReferenceWitnesses);
            capture_references_buffer(
                memory,
                base,
                game_state,
                wings,
                registered,
                Buffer::Growing(&mut self.handles),
                Buffer::Growing(&mut self.references.rows),
            )
            .unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[8] = elapsed(begin);
        let begin = qpc();
        if !self.active[10] {
            self.sorties.fail(SKIPPED, SKIPPED);
        } else if let Outcome::Rows { len } = self.references.outcome {
            let _reads = read_accounting::table(TableId::AirMissionInputs);
            match registered_wings(wings, registered) {
                Ok(missions) => self.sorties.capture_growing(
                    memory,
                    base,
                    game_state,
                    self.active[14],
                    missions,
                    &self.references.rows[..len],
                ),
                Err(reason) => self.sorties.fail(
                    Outcome::Unavailable { reason },
                    failed(self.active[14], reason),
                ),
            }
        } else {
            let reason = "air sortie inputs lack the complete base witnesses";
            self.sorties.fail(
                Outcome::Unavailable { reason },
                failed(self.active[14], reason),
            );
        }
        ticks[10] = elapsed(begin);
        let begin = qpc();
        self.mines.outcome = if !self.active[9] {
            SKIPPED
        } else {
            let _reads = read_accounting::table(TableId::NavalMineLevels);
            capture_mines_buffer(
                memory,
                base,
                game_state,
                wings,
                registered,
                Buffer::Growing(&mut self.handles),
                Buffer::Growing(&mut self.mines.rows),
            )
            .unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[9] = elapsed(begin);
        if !self.active[16] {
            self.scores.outcome = SKIPPED;
            scores.invalidate();
        } else {
            let begin = qpc();
            let _reads = read_accounting::table(TableId::RaidAirScores);
            if force_full {
                scores.invalidate();
            }
            // Only a raised capacity allocates, on this pool worker.
            let [regions, countries] = self.score_capacity;
            if scores
                .grow(regions, countries, self.scores.rows.len())
                .is_err()
            {
                self.scores.fail(Outcome::Unavailable {
                    reason: "collector allocation failed",
                });
                return ticks;
            }
            let walked = engine::raid_air_scores::walk_growing(
                memory,
                base,
                game_state,
                hour,
                scores,
                &mut self.scores.rows,
            );
            self.score_demand = [walked.regions, walked.countries];
            self.scores.outcome = bounded(walked.outcome, self.scores.rows.len());
            ticks[16] = elapsed(begin);
        }
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let sorties = self.sorties.outcomes();
        [
            self.equipment.outcome,
            self.bases.outcome,
            self.cumulative.outcome,
            self.history.outcome,
            self.damage.outcome,
            self.aces.outcome,
            self.statistics.outcome,
            self.registrations.outcome,
            self.references.outcome,
            self.mines.outcome,
            sorties[0],
            sorties[1],
            sorties[2],
            sorties[3],
            sorties[4],
            self.patrol_combats.outcome,
            self.scores.outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        let [missions, containers, modifiers, special_bases, metadata] = self.sorties.tables();
        [
            table::<tables::WingEquipment, _>(outcomes[0], &self.equipment.rows, Clone::clone),
            table::<tables::AirBases, _>(outcomes[1], &self.bases.rows, |r| rows::AirBaseRow {
                id: r.id,
                state: r.state,
                province: r.province,
                host: r.host,
                capacity: r.capacity,
                kind: r.kind,
                carrier: self.carrier.packet(r.id),
            }),
            table::<tables::WingCumulative, _>(outcomes[2], &self.cumulative.rows, |r| {
                rows::WingCumulativeRow {
                    id: r.id,
                    accidents: r.accidents,
                    experience: r.experience,
                }
            }),
            history_table(outcomes[3], &self.history.rows),
            damage_table(outcomes[4], &self.damage.rows),
            self.ace_table(outcomes[5]),
            table::<tables::AirStatistics, _>(outcomes[6], &self.statistics.rows, |r| {
                rows::AirStatisticsRow {
                    country: r.country,
                    region: r.region,
                    year_slot: r.year_slot,
                    year_window_wrapped: r.year_window_wrapped,
                    channels: r.channels,
                }
            }),
            self.registration_table(),
            air_reference_witness::table_view(
                outcomes[8],
                &self.references.rows[..outcomes[8].len().min(self.references.rows.len())],
                match outcomes[1] {
                    Outcome::Rows { len } => self.bases.rows.get(..len),
                    _ => None,
                },
            ),
            table::<tables::NavalMineLevels, _>(outcomes[9], &self.mines.rows, Clone::clone),
            missions,
            containers,
            modifiers,
            special_bases,
            metadata,
            table::<tables::AirNavalPatrolCombats, _>(
                outcomes[15],
                &self.patrol_combats.rows,
                Clone::clone,
            ),
            table::<tables::RaidAirScores, _>(outcomes[16], &self.scores.rows, |r| {
                rows::RaidAirScoreRow {
                    country: r.country,
                    region: r.region,
                    friendly: r.friendly,
                    enemy: r.enemy,
                }
            }),
        ]
    }

    fn registration_table(&self) -> TableView {
        if allocation::capture_stopped() {
            return unavailable("");
        }
        if let Outcome::Unavailable { reason } = self.registrations.outcome
            && let Some(country) = self.registration_failure.country_slot
        {
            let reason = match self.registration_failure.region {
                Some(region) => format!("{reason} (country slot {country}, region {region})"),
                None => format!("{reason} (country slot {country})"),
            };
            return unavailable(&reason);
        }
        table::<tables::AirRegionalMissions, _>(
            self.registrations.outcome,
            &self.registrations.rows,
            |r| rows::AirRegionalMissionRow {
                wing: r.wing,
                ordinal: r.ordinal,
            },
        )
    }

    fn ace_table(&self, outcome: Outcome) -> TableView {
        if self.names.required < self.names.len
            || !valid_names(outcome, &self.aces.rows, &self.names.first, self.names.len)
        {
            return unavailable("ace name range exceeds captured arena");
        }
        table::<tables::AirAces, _>(outcome, &self.aces.rows, |r| {
            let text = |range: cace::TextRange| {
                allocation::owned_vec(allocation::copy_vec(
                    &self.names.first[range.start..range.start + range.len],
                ))
            };
            rows::AirAceRow {
                id: r.id,
                country: r.country,
                ordinal: r.ordinal,
                aircraft_type_mask: r.aircraft_type_mask,
                assigned_wing: r.assigned_wing,
                alive: r.alive,
                handled: r.handled,
                kill_type: r.kill_type,
                death: r.death.map(|death| rows::AceDeath {
                    killer_ace: death.killer_ace,
                    killer_country: death.killer_country,
                }),
                name: text(r.name),
                surname: text(r.surname),
                callsign: text(r.callsign),
                presentation: rows::Recorded::Value(r.presentation.map(|value| {
                    rows::AirAcePresentation {
                        gender: value.gender,
                        pool_index: value.pool_index,
                        definition: value.definition.map(text),
                        portrait: value.portrait.map(|value| rows::AirAcePortraitInputs {
                            cached: value.cached.map(text),
                            culture: value.culture.map(text),
                            fallback: value.fallback.map(text),
                        }),
                    }
                })),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "air walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "air walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut [R]) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
}

/// The wing census sorted by handle and the registrations, both complete,
/// for the joins that look wings up by handle.
fn joins<'a>(
    identities: &'a mut Arena<WingIdentity>,
    identity_outcome: Outcome,
    registrations: &'a CaptureRows<cair_regional_mission::Row>,
) -> (&'a [WingIdentity], Option<&'a [cair_regional_mission::Row]>) {
    match (identity_outcome, registrations.outcome) {
        (Outcome::Rows { len: wings }, Outcome::Rows { len }) => {
            let identities = &mut identities.first[..wings];
            identities.sort_unstable_by_key(|wing| wing.wing);
            (&*identities, Some(&registrations.rows[..len]))
        }
        _ => (&[][..], None),
    }
}

/// Each registration's wing; `wings` is sorted by handle.
fn registered_wings<'a>(
    wings: &'a [WingIdentity],
    registered: Option<&'a [cair_regional_mission::Row]>,
) -> Result<impl Iterator<Item = Result<&'a WingIdentity, &'static str>>, &'static str> {
    let registered = registered.ok_or("air mission joins lack the complete registration census")?;
    Ok(registered.iter().map(move |registration| {
        wings
            .binary_search_by_key(&registration.wing, |wing| wing.wing)
            .map(|index| &wings[index])
            .map_err(|_| "registered mission missing from the wing census")
    }))
}

/// Registered regions supply conventional outcomes independently of the
/// naval-combat census. Its own unavailable table already refuses naval draws.
#[cfg(test)]
fn scenario_history_regions(
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    combats: Result<(Outcome, (usize, usize)), &'static str>,
    handles: &mut [u64],
) -> Result<usize, &'static str> {
    scenario_history_regions_buffer(wings, registered, combats, Buffer::Fixed(handles))
}

fn scenario_history_regions_buffer(
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    combats: Result<(Outcome, (usize, usize)), &'static str>,
    mut handles: Buffer<'_, u64>,
) -> Result<usize, &'static str> {
    let mut count = match combats {
        Ok((_, (first, len))) => {
            handles.copy_within(first..first + len, 0);
            len
        }
        Err(_) => 0,
    };
    handles.ensure(
        count
            .checked_add(registered.map_or(0, <[_]>::len))
            .ok_or("air scenario history region count overflow")?,
    )?;
    for wing in registered_wings(wings, registered)? {
        if let Some(region) = wing?.region {
            let region = u64::from(region);
            if !handles[..count].contains(&region) {
                *handles
                    .get_mut(count)
                    .ok_or("air scenario history region scratch incomplete")? = region;
                count += 1;
            }
        }
    }
    handles[..count].sort_unstable();
    Ok(count)
}

/// Probe registered active pools, every idle pool regardless of retained
/// mission region, unregistered training pools and transfer bases. The idle
/// branch owns its pool demand even when registered, so at most two entries
/// per wing are needed. Each distinct nonzero host is then resolved once,
/// including stateful bases (`140F4E0E0`, `140FFDCD0`).
#[cfg(test)]
fn capture_references(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    handles: &mut [u64],
    output: &mut [BaseWitness],
) -> Result<Outcome, &'static str> {
    capture_references_buffer(
        memory,
        base,
        game_state,
        wings,
        registered,
        Buffer::Fixed(handles),
        Buffer::Fixed(output),
    )
}

fn capture_references_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    mut handles: Buffer<'_, u64>,
    mut output: Buffer<'_, BaseWitness>,
) -> Result<Outcome, &'static str> {
    handles.ensure(
        wings
            .len()
            .checked_mul(2)
            .and_then(|count| count.checked_add(registered.map_or(0, <[_]>::len)))
            .ok_or("air reference count overflow")?,
    )?;
    let mut required = 0;
    let mut want = |handle: u64| {
        if let Some(slot) = handles.get_mut(required) {
            *slot = handle;
        }
        required += 1;
    };
    for wing in registered_wings(wings, registered)? {
        let wing = wing?;
        if wing.air_base == 0 {
            return Err("registered wing has a null pool base");
        }
        if wing.mission_mask != 0 {
            want(wing.air_base);
        }
    }
    for wing in wings.iter().filter(|wing| {
        (wing.mission_mask == 0
            || (wing.region.is_none() && wing.mission_mask & rows::AIR_MISSION_TRAINING != 0))
            && wing.air_base != 0
    }) {
        want(wing.air_base);
    }
    for wing in wings.iter().filter(|wing| wing.base_override != 0) {
        want(wing.base_override);
    }
    if required > handles.len() {
        return Ok(Outcome::Truncated {
            len: 0,
            unread_rows: required.min(u32::MAX as usize) as u32,
        });
    }
    let probes = &mut handles[..required];
    probes.sort_unstable();
    let mut len = 0usize;
    let mut unread_rows = 0_u32;
    let mut previous = 0;
    for &handle in probes.iter() {
        if handle == previous {
            continue;
        }
        previous = handle;
        output.ensure(len.checked_add(1).ok_or("air reference count overflow")?)?;
        let reference = map_air_base_reference::read_base(memory, base, game_state, handle)?;
        let witness = BaseWitness {
            handle,
            base: reference,
            ..Default::default()
        };
        match output.get_mut(len) {
            Some(slot) => {
                *slot = witness;
                len += 1;
            }
            None => unread_rows = unread_rows.saturating_add(1),
        }
    }
    air_reference_witness::capture_hosts(&mut output[..len], |handle| {
        map_air_base_reference::read_host_owner(memory, base, handle)
    })?;
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

/// `CanLayMinesInRegion@140F65E40`: each registered mine-laying wing's
/// effective-country mines in its mission region, once per pair. `pairs` is
/// scratch with one entry per registration.
#[cfg(test)]
fn capture_mines(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    pairs: &mut [u64],
    output: &mut [rows::NavalMineLevelRow],
) -> Result<Outcome, &'static str> {
    capture_mines_buffer(
        memory,
        base,
        game_state,
        wings,
        registered,
        Buffer::Fixed(pairs),
        Buffer::Fixed(output),
    )
}

fn capture_mines_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    mut pairs: Buffer<'_, u64>,
    mut output: Buffer<'_, rows::NavalMineLevelRow>,
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    pairs.ensure(registered.map_or(0, <[_]>::len))?;
    let mut count = 0;
    for wing in registered_wings(wings, registered)? {
        let wing = wing?;
        let Some(region) = wing
            .region
            .filter(|_| wing.mission_mask & rows::AIR_MISSION_LAY_MINES != 0)
        else {
            continue;
        };
        let country = countries.resolve(wing.effective_country())?;
        *pairs
            .get_mut(count)
            .ok_or("mine pair scratch smaller than the registrations")? =
            u64::from(country.canonical_slot) << 32 | u64::from(region);
        count += 1;
    }
    output.ensure(count)?;
    let pairs = &mut pairs[..count];
    pairs.sort_unstable();
    let mut len = 0;
    let mut unread_rows = 0_u32;
    let mut previous = None;
    for &pair in pairs.iter() {
        if previous.replace(pair) == Some(pair) {
            continue;
        }
        let Some(slot) = output.get_mut(len) else {
            unread_rows = unread_rows.saturating_add(1);
            continue;
        };
        let (country, region) = ((pair >> 32) as u32, pair as u32);
        let country = countries
            .at(country)?
            .ok_or("mine-laying country slot is empty")?;
        *slot = rows::NavalMineLevelRow {
            country: country.canonical_slot,
            region,
            raw_mines: cstrategic_navy_manager::regional_operand(
                memory, base, game_state, country, region,
            )?,
        };
        len += 1;
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

/// `141237D20`'s first input for the registered naval-patrol missions: each
/// effective country's task forces in a naval combat in one of its patrolled
/// regions, in native order. `scratch` holds the (country, region) pairs and
/// then the regions those combats are in, whose span it returns.
fn capture_patrol_combats(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    mut scratch: Buffer<'_, u64>,
    combats: Buffer<'_, rows::AirNavalPatrolCombatRow>,
) -> Result<(Outcome, (usize, usize)), &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    scratch.ensure(registered.map_or(0, <[_]>::len))?;
    let mut count = 0;
    for wing in registered_wings(wings, registered)? {
        let wing = wing?;
        let Some(region) = wing
            .region
            .filter(|_| wing.mission_mask & rows::AIR_MISSION_NAVAL_PATROL != 0)
        else {
            continue;
        };
        let country = countries.resolve(wing.effective_country())?;
        *scratch
            .get_mut(count)
            .ok_or("naval-patrol scratch smaller than the registrations")? =
            u64::from(country.canonical_slot) << 32 | u64::from(region);
        count += 1;
    }
    patrol_combats_buffer(scratch, count, combats, |slot, emit| {
        let country = countries
            .at(slot)?
            .ok_or("naval-patrol country slot is empty")?;
        engine::air_kamikaze::task_force_combats(memory, base, country, emit)
    })
}

/// Sort the `count` pairs in `scratch`, walk each distinct country's task
/// forces through `walk`, keep those in a patrolled region with their
/// ordinal, and list the regions they are in after the pairs.
#[cfg(test)]
fn patrol_combats(
    scratch: &mut [u64],
    count: usize,
    combats: &mut [rows::AirNavalPatrolCombatRow],
    walk: impl FnMut(
        u32,
        &mut dyn FnMut(engine::air_kamikaze::TaskForceCombat) -> Result<(), &'static str>,
    ) -> Result<(), &'static str>,
) -> Result<(Outcome, (usize, usize)), &'static str> {
    patrol_combats_buffer(Buffer::Fixed(scratch), count, Buffer::Fixed(combats), walk)
}

fn patrol_combats_buffer(
    mut scratch: Buffer<'_, u64>,
    count: usize,
    mut combats: Buffer<'_, rows::AirNavalPatrolCombatRow>,
    mut walk: impl FnMut(
        u32,
        &mut dyn FnMut(engine::air_kamikaze::TaskForceCombat) -> Result<(), &'static str>,
    ) -> Result<(), &'static str>,
) -> Result<(Outcome, (usize, usize)), &'static str> {
    scratch.ensure(
        count
            .checked_mul(2)
            .ok_or("naval-patrol scratch count overflow")?,
    )?;
    if scratch.len() < count * 2 {
        return Err("naval-patrol scratch smaller than the registrations");
    }
    let (pairs, needed) = scratch.split_at_mut(count);
    pairs.sort_unstable();
    let (mut len, mut unread_rows, mut regions) = (0usize, 0_u32, 0usize);
    let mut first = 0;
    while first < pairs.len() {
        let slot = (pairs[first] >> 32) as u32;
        let last = first + pairs[first..].partition_point(|&pair| (pair >> 32) as u32 == slot);
        let patrolled = &pairs[first..last];
        first = last;
        let mut ordinal = 0;
        walk(slot, &mut |combat| {
            let key = u64::from(slot) << 32 | u64::from(combat.region);
            if patrolled.binary_search(&key).is_err() {
                return Ok(());
            }
            let region = u64::from(combat.region);
            if !needed[..regions].contains(&region) {
                needed[regions] = region;
                regions += 1;
            }
            combats.ensure(
                len.checked_add(1)
                    .ok_or("naval-patrol combat count overflow")?,
            )?;
            match combats.get_mut(len) {
                Some(row) => {
                    *row = rows::AirNavalPatrolCombatRow {
                        country: slot,
                        ordinal,
                        province: combat.province,
                        region: combat.region,
                    };
                    len += 1;
                }
                None => unread_rows = unread_rows.saturating_add(1),
            }
            ordinal += 1;
            Ok(())
        })?;
    }
    needed[..regions].sort_unstable();
    let outcome = if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    };
    Ok((outcome, (count, regions)))
}

pub(super) fn require_parent(parent: Outcome, child: Outcome) -> Outcome {
    match parent {
        Outcome::Rows { .. } => child,
        Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
        Outcome::Truncated { .. } => Outcome::Unavailable {
            reason: "shared air wing census truncated; dependent census incomplete",
        },
    }
}

fn append(first: Outcome, next: Outcome) -> Outcome {
    let count = |outcome| match outcome {
        Outcome::Rows { len } => Ok((len, 0_u32)),
        Outcome::Truncated { len, unread_rows } => Ok((len, unread_rows)),
        Outcome::Unavailable { reason } => Err(reason),
    };
    let result = (|| {
        let (len, omitted) = count(first)?;
        let (next_len, next_omitted) = count(next)?;
        let len = len.checked_add(next_len).ok_or("air row count overflow")?;
        let unread_rows = omitted
            .checked_add(next_omitted)
            .ok_or("air omitted row count overflow")?;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Read entries individually so only native references and amounts are retained;
/// the pointer used to authenticate each variant remains a local read context.
fn read_equipment(
    memory: &impl Memory,
    base: usize,
    source: &WingSource<'_>,
    output: &mut [rows::WingEquipmentRow],
) -> Outcome {
    read_equipment_buffer(memory, base, source, engine::walk::Buffer::Fixed(output), 0).0
}

fn read_equipment_buffer(
    memory: &impl Memory,
    base: usize,
    source: &WingSource<'_>,
    mut output: engine::walk::Buffer<'_, rows::WingEquipmentRow>,
    start: usize,
) -> (Outcome, usize) {
    let mut required = 0;
    let result = (|| {
        // CAirWing constructor 140F445E0 owns the pool at +0x1D8;
        // CEquipmentVariantPool::serialize@140FFDB00 walks +0x2C rows at
        // +0x20 with stride 0x10. Grow from that original header once.
        let entries = cair_wing::equipment_pool(memory, base, source.pointer)?;
        required = entries.count as usize;
        let end = start
            .checked_add(required)
            .ok_or("air equipment demand overflow")?;
        output.ensure(end)?;
        let len = output.len().saturating_sub(start).min(required);
        for (ordinal, destination) in output[start..start + len].iter_mut().enumerate() {
            let address = entries
                .element(ordinal as u32, equipment_entry::STRIDE)
                .ok_or("air equipment entry address overflow")?;
            let entry = equipment_entry::read(memory, base, address, ordinal as u32)?;
            *destination = rows::WingEquipmentRow {
                wing: source.row.id,
                ordinal: entry.source_ordinal,
                variant: entry.variant_handle,
                amount_raw: entry.amount_raw,
            };
        }
        Ok(if len == entries.count as usize {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: entries.count - len as u32,
            }
        })
    })();
    (
        result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        required,
    )
}

fn names_outcome(walk: cace::NameWalk, rows: &[cace::Row], bytes: &[u8]) -> Outcome {
    let outcome = bounded(walk.outcome, rows.len());
    if walk.required_bytes < walk.bytes_len || !valid_names(outcome, rows, bytes, walk.bytes_len) {
        Outcome::Unavailable {
            reason: "ace name range exceeds captured arena",
        }
    } else {
        outcome
    }
}

fn valid_names(outcome: Outcome, rows: &[cace::Row], bytes: &[u8], len: usize) -> bool {
    len <= bytes.len()
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                [
                    Some(row.name),
                    Some(row.surname),
                    Some(row.callsign),
                    row.presentation.and_then(|value| value.definition),
                    row.presentation
                        .and_then(|value| value.portrait)
                        .and_then(|value| value.cached),
                    row.presentation
                        .and_then(|value| value.portrait)
                        .and_then(|value| value.culture),
                    row.presentation
                        .and_then(|value| value.portrait)
                        .and_then(|value| value.fallback),
                ]
                .into_iter()
                .flatten()
                .all(|range| {
                    range
                        .start
                        .checked_add(range.len)
                        .is_some_and(|end| end <= len)
                })
            })
        })
}

/// A partial native entry stream cannot name the exact count of missing grouped
/// parents, and its last parent may be partial. Keep that hour unavailable with
/// a specific capacity reason; raw capture truncation still grows the pool.
fn grouped_source<R>(outcome: Outcome, rows: &[R]) -> Result<&[R], TableView> {
    if allocation::capture_stopped() {
        return Err(unavailable(""));
    }
    match bounded(outcome, rows.len()) {
        Outcome::Rows { len } => Ok(&rows[..len]),
        Outcome::Truncated { len, unread_rows } => Err(unavailable(&format!(
            "air regional entry capacity: {len} captured, {unread_rows} omitted; grouped table incomplete"
        ))),
        Outcome::Unavailable { reason } => Err(unavailable(reason)),
    }
}

fn history_table(outcome: Outcome, captured: &[sair_wing_combat_data::Row]) -> TableView {
    let captured = match grouped_source(outcome, captured) {
        Ok(rows) => rows,
        Err(table) => return table,
    };
    let chunks =
        || captured.chunk_by(|a, b| (a.key.country, a.key.region) == (b.key.country, b.key.region));
    // The shared table helper rejects a repeated outer parent separated by a
    // different parent. Native duplicate children within a parent remain exact.
    table_iter::<tables::AirRegionHistory, _>(
        Outcome::Rows {
            len: chunks().count(),
        },
        chunks(),
        |group| rows::AirRegionHistoryRow {
            country: group[0].key.country,
            region: group[0].key.region,
            entries: allocation::owned_vec(allocation::collect_vec(group.iter().map(|row| {
                rows::AirHistoryEntry {
                    side: row.key.side,
                    date: row.key.date,
                    wing_id: row.key.wing_id,
                    classification: row.key.classification,
                    region_owner: row.region_owner,
                    planes: row.planes,
                    mission_mask: row.mission_mask,
                    selection: row.selection,
                    copied: row.copied,
                    attributed_country: row.attributed_country,
                }
            }))),
        },
    )
}

fn damage_table(outcome: Outcome, captured: &[sdamage_entry::Row]) -> TableView {
    let captured = match grouped_source(outcome, captured) {
        Ok(rows) => rows,
        Err(table) => return table,
    };
    let chunks = || {
        captured.chunk_by(|a, b| {
            (a.record.country, a.record.region) == (b.record.country, b.record.region)
        })
    };
    table_iter::<tables::AirRegionDamage, _>(
        Outcome::Rows {
            len: chunks().count(),
        },
        chunks(),
        |group| rows::AirRegionDamageRow {
            country: group[0].record.country,
            region: group[0].record.region,
            entries: allocation::owned_vec(allocation::collect_vec(group.iter().map(|row| {
                rows::AirDamageEntry {
                    side: row.record.side,
                    date: row.record.date,
                    wing_id: row.record.wing_id,
                    classification: row.record.classification,
                    channel: row.channel,
                    cause: row.cause,
                    amount: row.amount,
                }
            }))),
        },
    )
}

/// One complete source census, independent of the bounded map projection.
/// All transient addresses stay owned opaque scalars and are consumed only
/// during acquisition; the worker installs value rows without memory reads.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WingMembershipRead {
    pub value: engine::ctheatre::WingMembership,
    pub error: Option<&'static str>,
}

mod wing;
#[cfg(test)]
use wing::WING_PACK_N;
#[cfg(test)]
pub(crate) use wing::copy_air_refresh;
pub(crate) use wing::{
    AirWingCopyContext, AirWingCopyReport, AirWingJob, copy_air_refresh_growing, decode_air_payload,
};

mod stats;
#[cfg(test)]
pub(crate) use stats::copy_air_stats_refresh;
#[cfg(test)]
use stats::{AIR_STATS_N, STATS_MASK, copy_stats_channels, sspan};
pub(crate) use stats::{
    AirStatsCopyContext, AirStatsCopyReport, AirStatsJob, copy_air_stats_refresh_growing,
    decode_air_stats_payload,
};

mod bases;
pub(super) mod carrier;
pub(crate) use carrier::capture_status as carrier_capture_status;
mod carrier_memory;
use super::fog;
#[cfg(test)]
pub(crate) use bases::copy_air_bases_refresh;
pub(crate) use bases::{
    AIR_BASES_N, AirBasesCopyContext, AirBasesCopyReport, AirBasesJob, AirBasesMemo,
    AirBasesMemoContext, AirBasesMemoReport, air_bases_memo_memory, copy_air_bases_refresh_growing,
};
#[cfg(test)]
use bases::{AIR_REGISTRATIONS_MASK, bases_pool_u32, bspan, copy_air_bases_pass, resolve_link};

mod history;
#[cfg(test)]
use history::acquire_air_history_pass;
#[cfg(test)]
pub(crate) use history::copy_air_history_refresh;
pub(crate) use history::{
    AirHistoryCopyContext, AirHistoryCopyReport, AirHistoryJob, copy_air_history_refresh_growing,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transport_witness_tests;

#[cfg(test)]
mod fixed_mirror_gather_tests;

/// Shared synthetic address space for the wing-pack copy/decode harness:
/// one country, one pool, two wings with equipment entries and cumulative
/// counters. Mirrors the engine wing/equipment fixture shapes.
#[cfg(test)]
pub(crate) mod wing_test_kit;

/// Shared synthetic address space for the air-statistics copy/decode
/// harness: one owner with a two-region history table, each region holding
/// one history with a valid yearly window, record and channel values.
/// Mirrors the engine statistics fixture shapes contiguously.
#[cfg(test)]
pub(crate) mod stats_test_kit;

#[cfg(test)]
mod air_bases_memo_tests;

/// Shared synthetic address space for the bases/registrations/aces
/// copy/decode harness: two countries, a manager with two state bases
/// (one indexed hole) plus one stateless host, per-base containers with
/// pools and wings, and two aces (one alive with inline names, one dead
/// with heap names). Mirrors the engine bases/registrations/aces fixture
/// shapes.
#[cfg(test)]
pub(crate) mod air_bases_test_kit;

/// Shared synthetic address space for the history/damage copy/decode
/// harness: one owner with two region slots (one null), three records with
/// damage vectors on two of them. Mirrors the engine region-combat fixture
/// shapes.
#[cfg(test)]
pub(crate) mod air_history_test_kit;

#[cfg(test)]
#[path = "air_mission_scale_tests.rs"]
mod mission_scale_tests;

#[cfg(test)]
#[path = "air/sizing_tests.rs"]
mod sizing_tests;
