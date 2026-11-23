use collector_memory::read_accounting;
use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
};
use collector_projection::table_conversion::{table, unavailable};
use collector_raw::raw::RawLog;
use engine_air_history::cloop_history;
use engine_allocation as allocation;
use engine_map_air::map_air_base_reference;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    walk::{Buffer, Outcome},
};
use engine_native_parts::{cair_wing_pool, equipment_entry};
use engine_native::{ccountry, cgame_state, cprovince, cstate};
use engine_native_air::{
    cace,
    cair_base,
    cair_regional_mission::{self, WingIdentity},
    cair_wing,
    ccountry_air_container,
    cstrategic_air_manager,
};
use replay_data::rows::{self, TableId, tables};
use replay_frame::frame::{TableInput, TableView};

use collector_air_histories::air_histories;
use collector_air_sorties::{
    air_reference_witness::{self, BaseWitness},
    air_sorties::{self, Sorties},
};

pub struct MissionSources<'a, C> {
    pub parent: Outcome,
    pub histories: &'a mut [air_histories::Staging],
    pub changes: &'a C,
    pub carry: Option<MissionCarry<'a>>,
}

pub struct MissionCarry<'a> {
    pub store: &'a mut MissionStore,
    pub changes: &'a dyn collector_missions::mission_changes::Changes,
}

pub const N: usize = 16;
pub const SIZED: usize = 9;
pub const TABLES: [TableId; N] = [
    TableId::WingEquipment,
    TableId::AirBases,
    TableId::WingCumulative,
    TableId::AirAces,
    TableId::AirStatistics,
    TableId::AirRegionalMissions,
    TableId::AirReferenceWitnesses,
    TableId::AirMissionInputs,
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
    TableId::AirSortieModifierMetadata,
    TableId::AirNavalPatrolCombats,
    TableId::RaidAirScores,
    TableId::AirGroundCrews,
    TableId::AirTransferInputs,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; SIZED],
    pub ace_names: usize,
    pub wings: usize,
    pub sorties: air_sorties::Capacities,
    pub scores: ScoreCapacities,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    ace_names,
    wings,
    sorties,
    scores
});

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ScoreCapacities {
    pub rows: usize,
    pub regions: usize,
    pub countries: usize,
}

collector_primitives::capacity_max!(ScoreCapacities {
    rows,
    regions,
    countries
});

impl Capacities {
    pub fn presize_missions(
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
        engine_native_air::cstrategic_air::visit(memory, base, state, |root| {
            let active = root
                .address
                .checked_add(engine_native_air::cstrategic_air::ALL_ACTIVE_MISSIONS)
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
        self.rows[5] = self.rows[5].max(missions);
        self.rows[7] = 0;
        self.scores.countries = self.scores.countries.max(countries);
        self.scores.rows = self.scores.rows.max(countries);
        let wings =
            collector_extended_support::demand::rows(cair_wing::count(memory, base, state))?;
        let handles = wings
            .checked_mul(2)
            .ok_or("air reference sizing count overflow")?;
        self.wings = self.wings.max(wings).max(missions);
        self.rows[6] = self.rows[6].max(handles);
        let task_forces = engine_native::cfleet::count(memory, base, state);
        self.rows[8] = self.rows[8].max(collector_extended_support::demand::rows(
            task_forces.task_forces,
        )?);
        Ok(())
    }

    pub fn raise_for_names(&mut self, demand: crate::sizing::AceNamesDemand) {
        self.rows[3] = self.rows[3].max(demand.aces);
        self.ace_names = self.ace_names.max(demand.ace_bytes);
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        collector_primitives::country_slots::raise(&mut self.scores.countries, slots);
        collector_primitives::country_slots::raise(&mut self.scores.rows, slots);
    }
}

pub struct WingSource<'a> {
    pub pointer: usize,
    pub row: &'a cair_wing::Row,
}

pub struct ScorePart {
    rows: CaptureRows<engine_raid_inputs::raid_air_scores::Row>,
    demand: [usize; 2],
    ticks: u64,
}

impl ScorePart {
    pub fn vacant() -> Self {
        Self {
            rows: CaptureRows::new(0),
            demand: [0; 2],
            ticks: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        capacity: [usize; 2],
        held: &mut engine_raid_inputs::raid_air_scores::Held,
    ) -> u64 {
        self.rows.reset();
        self.demand = [0; 2];
        self.ticks = 0;
        if !replay_data::cadence::should_walk(TableId::RaidAirScores, hour, force_full) {
            self.rows.outcome = SKIPPED;
            held.invalidate();
            return 0;
        }
        let begin = qpc();
        let _reads = read_accounting::table(TableId::RaidAirScores);
        if force_full {
            held.invalidate();
        }
        held.certify(collector_observers::raid_changes::air_clear_token());
        let [regions, countries] = capacity;
        if held.grow(regions, countries, self.rows.rows.len()).is_err() {
            self.rows.fail(Outcome::Unavailable {
                reason: "collector allocation failed",
            });
            return 0;
        }
        let walked = engine_raid_inputs::raid_air_scores::walk(
            memory,
            base,
            game_state,
            hour,
            held,
            &mut self.rows.rows,
        );
        self.demand = [walked.regions, walked.countries];
        self.rows.outcome = bounded(walked.outcome, self.rows.rows.len());
        self.ticks = elapsed(begin);
        self.ticks
    }

    pub fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows.reset();
        self.demand = [0; 2];
        self.ticks = 0;
        self.rows.fail(failed(
            replay_data::cadence::should_walk(TableId::RaidAirScores, hour, force_full),
            reason,
        ));
    }
}

pub struct Captured {
    equipment: CaptureRows<rows::WingEquipmentRow>,
    bases: CaptureRows<cair_base::Row>,
    pub carrier: carrier::Bases,
    cumulative: CaptureRows<cair_wing::CumulativeRow>,
    aces: CaptureRows<cace::Row>,
    statistics: CaptureRows<cloop_history::Row>,
    scores: CaptureRows<engine_raid_inputs::raid_air_scores::Row>,
    score_capacity: [usize; 2],
    score_demand: [usize; 2],
    scores_early: Option<u64>,
    names: Arena<u8>,
    pub identities: Arena<WingIdentity>,
    pub identity_outcome: Outcome,
    identities_owned: bool,
    registrations: CaptureRows<cair_regional_mission::Row>,
    mission_order: Vec<mission_store::OrderLabel>,
    registration_failure: cair_regional_mission::FailureContext,
    handles: Vec<u64>,
    references: CaptureRows<BaseWitness>,
    sorties: Sorties,
    ground_crews: CaptureRows<rows::AirGroundCrewRow>,
    ground_demand: Vec<ground_crews::Demand>,
    ground_merge: Vec<rows::AirGroundCrewRow>,
    supply: engine_map_air::map_air_supply::Capture,
    supply_ok: bool,
    ground_lenient: bool,
    transfers: CaptureRows<rows::AirTransferInputRow>,
    transfer_demand: Vec<usize>,
    transfer_ticks: u64,
    patrol_combats: CaptureRows<rows::AirNavalPatrolCombatRow>,
    missions: Option<[u64; 2]>,
    joins: Option<[u64; 3]>,
    identity_ticks: u64,
    active: [bool; N],
    pub stats_raw_mode: bool,
    pub bases_raw_mode: bool,
}

pub const NO_WING_SOURCE: Outcome = Outcome::Unavailable {
    reason: "shared air wing census not supplied",
};

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay_data::cadence::SKIPPED_REASON,
};

const BASES_NOT_STAGED: Outcome = Outcome::Unavailable {
    reason: "air bases payload not staged this hour",
};

const STATISTICS_NOT_STAGED: Outcome = Outcome::Unavailable {
    reason: "air statistics payload not staged this hour",
};

fn failed(active: bool, reason: &'static str) -> Outcome {
    if active {
        Outcome::Unavailable { reason }
    } else {
        SKIPPED
    }
}

impl Captured {
    pub fn fail_late_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        if self.scores_early.take().is_none() {
            self.scores.fail(failed(self.active[13], reason));
        }
        self.equipment.fail(failed(self.active[0], reason));
        self.cumulative.fail(failed(self.active[2], reason));
        if self.missions.is_none() {
            self.fail_missions_rows(reason);
        }
        if self.joins.is_none() {
            self.references.fail(failed(self.active[6], reason));
            self.sorties.fail(
                failed(self.active[7], reason),
                failed(self.active[11], reason),
            );
            self.ground_crews.fail(failed(self.active[14], reason));
            self.transfers.fail(failed(self.active[15], reason));
        }
        if !self.bases_raw_mode {
            self.bases.fail(failed(self.active[1], reason));
            self.aces.fail(failed(self.active[3], reason));
        }
        if !self.stats_raw_mode {
            self.statistics.fail(failed(self.active[4], reason));
        }
    }

    pub fn vacant() -> Self {
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
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            equipment: CaptureRows::new(capacity.rows[0]),
            bases: CaptureRows::new(capacity.rows[1]),
            carrier: carrier::Bases::new(capacity.rows[1]),
            cumulative: CaptureRows::new(capacity.rows[2]),
            aces: CaptureRows::new(capacity.rows[3]),
            statistics: CaptureRows::new(capacity.rows[4]),
            scores: CaptureRows::new(capacity.scores.rows),
            score_capacity: [capacity.scores.regions, capacity.scores.countries],
            score_demand: [0; 2],
            scores_early: None,
            names: Arena::new(capacity.ace_names),
            identities: Arena::new(capacity.wings),
            identity_outcome: NO_WING_SOURCE,
            identities_owned: false,
            registrations: CaptureRows::new(capacity.rows[5]),
            mission_order: Vec::new(),
            registration_failure: Default::default(),
            handles: allocation::owned_vec(allocation::filled(capacity.wings.saturating_mul(2), 0)),
            references: CaptureRows::new(capacity.rows[6]),
            sorties: Sorties::with_capacities(&capacity.sorties),
            ground_crews: CaptureRows::new(capacity.wings),
            ground_demand: allocation::owned_vec(allocation::default_vec(capacity.wings)),
            ground_merge: Vec::new(),
            supply: Default::default(),
            supply_ok: false,
            ground_lenient: false,
            transfers: CaptureRows::new(capacity.wings),
            transfer_demand: Vec::new(),
            transfer_ticks: 0,
            patrol_combats: CaptureRows::new(capacity.rows[8]),
            missions: None,
            joins: None,
            identity_ticks: 0,
            active: [true; N],
            stats_raw_mode: false,
            bases_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.equipment.reset();
        self.equipment.outcome = NO_WING_SOURCE;
        self.bases.reset();
        self.carrier.reset();
        self.cumulative.reset();
        self.cumulative.outcome = NO_WING_SOURCE;
        self.aces.reset();
        self.statistics.reset();
        self.scores.reset();
        self.score_demand = [0; 2];
        self.names.reset();
        self.identities.reset();
        self.identity_outcome = NO_WING_SOURCE;
        self.identities_owned = false;
        self.reset_missions();
        self.identity_ticks = 0;
    }

    fn reset_missions(&mut self) {
        self.registrations.reset();
        self.mission_order.clear();
        self.registration_failure = Default::default();
        self.patrol_combats.reset();
        self.missions = None;
        self.joins = None;
        self.reset_joins();
    }

    fn reset_joins(&mut self) {
        self.transfers.reset();
        self.transfer_demand.clear();
        self.transfer_ticks = 0;
        self.references.reset();
        self.sorties.reset();
        self.ground_crews.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.equipment.allocated_bytes()
            + self.bases.allocated_bytes()
            + self.carrier.bytes()
            + self.cumulative.allocated_bytes()
            + self.aces.allocated_bytes()
            + self.statistics.allocated_bytes()
            + self.scores.allocated_bytes()
            + self.names.allocated_bytes()
            + self.identities.allocated_bytes()
            + self.registrations.allocated_bytes()
            + self.mission_order.capacity() * size_of::<mission_store::OrderLabel>()
            + self.handles.capacity() * size_of::<u64>()
            + self.references.allocated_bytes()
            + self.sorties.allocated_bytes()
            + self.ground_crews.allocated_bytes()
            + self.transfers.allocated_bytes()
            + self.transfer_demand.capacity() * size_of::<usize>()
            + self.ground_demand.capacity() * size_of::<ground_crews::Demand>()
            + self.supply.allocated_bytes()
            + self.ground_merge.capacity() * std::mem::size_of::<rows::AirGroundCrewRow>()
            + self.patrol_combats.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.equipment.grow(&mut capacity.rows[0]);
        self.bases.grow(&mut capacity.rows[1]);
        collector_primitives::table_capture::grow_vec(&mut self.carrier.rows, capacity.rows[1]);
        self.carrier.grow();
        self.cumulative.grow(&mut capacity.rows[2]);
        self.aces.grow(&mut capacity.rows[3]);
        self.statistics.grow(&mut capacity.rows[4]);
        self.scores.grow(&mut capacity.scores.rows);
        let scores = &mut capacity.scores;
        collector_primitives::table_capture::grow_capacity(
            &mut scores.regions,
            self.score_demand[0],
        );
        collector_primitives::table_capture::grow_capacity(
            &mut scores.countries,
            self.score_demand[1],
        );
        self.score_capacity = [scores.regions, scores.countries];
        self.names.grow(&mut capacity.ace_names);
        self.identities.grow(&mut capacity.wings);
        collector_primitives::table_capture::grow_vec(
            &mut self.handles,
            capacity.wings.saturating_mul(2).max(capacity.rows[6]),
        );
        self.registrations.grow(&mut capacity.rows[5]);
        self.references.grow(&mut capacity.rows[6]);
        self.sorties.grow(&mut capacity.sorties);
        self.ground_crews.grow(&mut capacity.wings);
        collector_primitives::table_capture::grow_vec(&mut self.ground_demand, capacity.wings);
        self.patrol_combats.grow(&mut capacity.rows[8]);
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active =
            TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
    }

    pub fn begin_supply_hour(&mut self) {
        self.supply_ok = false;
    }

    pub fn install_supply(
        &mut self,
        capture: &mut engine_map_air::map_air_supply::Capture,
        ok: bool,
    ) {
        std::mem::swap(&mut self.supply, capture);
        self.supply_ok = ok;
    }

    pub fn merge_ground_supply(&mut self, memory: &impl Memory, base: usize) {
        if !self.supply_ok || !self.active[14] {
            return;
        }
        let Outcome::Rows { len } = self.ground_crews.outcome else {
            return;
        };
        let begin = qpc();
        let _reads = read_accounting::table(TableId::AirGroundCrews);
        self.ground_crews.rows.truncate(len);
        let (merged, stable) = collector_missions::ground_crew_changes::with_flags(
            memory,
            base,
            self.supply.rows.len(),
            |flags| {
                merge_supply(
                    &mut self.ground_crews.rows,
                    &mut self.ground_merge,
                    &self.supply.rows,
                    |country, region| flags.get(country, region),
                )
            },
        );
        self.ground_crews.outcome = match merged {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(()) if stable || self.ground_lenient => Outcome::Rows {
                len: self.ground_crews.rows.len(),
            },
            Ok(()) => Outcome::Unavailable {
                reason: "Ground Crews writer certificate changed during acquisition",
            },
        };
        if let Some(joins) = self.joins.as_mut() {
            joins[2] = joins[2].saturating_add(elapsed(begin));
        }
    }

    pub fn missions_captured(&self) -> bool {
        self.missions.is_some()
    }

    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// when carrying mission reuse so global `mission_changes` demand updates belong to this pass.
    pub unsafe fn capture_missions(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        sources: MissionSources<'_, impl collector_air_histories::history_changes::Changes>,
    ) {
        let MissionSources {
            parent,
            histories,
            changes,
            carry,
        } = sources;
        if !self.identities_owned {
            self.identity_outcome = require_parent(parent, self.identity_outcome);
        }
        self.registrations.reset();
        self.registration_failure = Default::default();
        self.patrol_combats.reset();
        self.mission_order.clear();
        let begin = qpc();
        self.registrations.outcome = if !self.active[5] {
            SKIPPED
        } else if let Outcome::Rows { len } = self.identity_outcome {
            let _reads = read_accounting::table(TableId::AirRegionalMissions);
            let identities = &mut self.identities.first[..len];
            identities.sort_unstable_by_key(|wing| wing.mission_pointer);
            let original = match carry {
                Some(MissionCarry { store, changes }) => {
                    // SAFETY: Caller holds hourly capture admission for this mission carry pass.
                    let mut reuse = unsafe { MissionReuse::begin(store, changes) };
                    let walked = cair_regional_mission::walk_carried(
                        memory,
                        base,
                        game_state,
                        identities,
                        Buffer::Growing(&mut self.registrations.rows),
                        &mut self.registration_failure,
                        &mut reuse,
                    );
                    // SAFETY: Caller holds hourly capture admission for this mission carry pass.
                    unsafe {
                        reuse.finish_with_order(
                            matches!(walked, Outcome::Rows { .. }),
                            memory,
                            identities,
                            &mut self.mission_order,
                        );
                    }
                    walked
                }
                None => cair_regional_mission::walk_with_diagnostic(
                    memory,
                    base,
                    game_state,
                    identities,
                    &mut self.registrations.rows,
                    &mut self.registration_failure,
                ),
            };
            bounded(original, self.registrations.rows.len())
        } else {
            Outcome::Unavailable {
                reason: "registered missions lack the complete wing census",
            }
        };
        let registered_ticks = elapsed(begin).saturating_add(self.identity_ticks);
        let begin = qpc();
        let (wings, registered) = joins(
            &mut self.identities,
            self.identity_outcome,
            &self.registrations,
        );
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
            _ if !self.active[12] => SKIPPED,
            Ok((outcome, _)) => outcome,
            Err(reason) => Outcome::Unavailable { reason },
        };
        air_histories::supply(histories, changes, &mut air_histories::opening());
        self.missions = Some([registered_ticks, elapsed(begin)]);
    }

    pub fn join_missions(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        store: Option<&mut MissionStore>,
    ) {
        self.joins = Some(self.capture_joins(memory, base, game_state));
        let begin = qpc();
        let _reads = read_accounting::table(TableId::AirTransferInputs);
        self.transfers.outcome = (|| {
            if !self.active[15] {
                return Ok(SKIPPED);
            }
            let store = store.ok_or("Air transfer writer store unavailable")?;
            let wings = match self.identity_outcome {
                Outcome::Rows { len } => &self.identities.first[..len],
                _ => return Err("Air transfers lack the complete wing census"),
            };
            let witnesses = match self.references.outcome {
                Outcome::Rows { len } => &self.references.rows[..len],
                _ => return Err("Air transfers lack the complete base witnesses"),
            };
            let result = store.transfers.capture(
                memory,
                wings,
                &self.transfer_demand,
                witnesses,
                &mut self.transfers.rows,
            );
            store.measure_transfers();
            result?;
            Ok(Outcome::Rows {
                len: self.transfers.rows.len(),
            })
        })()
        .unwrap_or_else(|reason| Outcome::Unavailable { reason });
        self.transfer_ticks = elapsed(begin);
    }

    fn capture_joins(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; 3] {
        self.reset_joins();
        let mut ticks = [0; 3];
        let (wings, registered) = joins(
            &mut self.identities,
            self.identity_outcome,
            &self.registrations,
        );
        let begin = qpc();
        self.references.outcome = if !self.active[6] {
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
                Some(&mut self.transfer_demand),
            )
            .unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[0] = elapsed(begin);
        let begin = qpc();
        self.ground_lenient = registered.is_some_and(<[_]>::is_empty);
        let ((ground_outcome, ground_ticks), stable) =
            collector_missions::ground_crew_changes::with_flags(
                memory,
                base,
                wings.len(),
                |flags| {
                    if !self.active[7] {
                        self.sorties.fail(SKIPPED, SKIPPED);
                    } else if let Outcome::Rows { len } = self.references.outcome {
                        let _reads = read_accounting::table(TableId::AirMissionInputs);
                        match registered_wings(wings, registered) {
                            Ok(missions) => self.sorties.capture_observed(
                                memory,
                                base,
                                game_state,
                                self.active[11],
                                missions,
                                &self.references.rows[..len],
                                flags,
                            ),
                            Err(reason) => self.sorties.fail(
                                Outcome::Unavailable { reason },
                                failed(self.active[11], reason),
                            ),
                        }
                    } else {
                        let reason = "air sortie inputs lack the complete base witnesses";
                        self.sorties.fail(
                            Outcome::Unavailable { reason },
                            failed(self.active[11], reason),
                        );
                    }
                    let begin_ground = qpc();
                    let outcome = if !self.active[14] {
                        SKIPPED
                    } else {
                        let _reads = read_accounting::table(TableId::AirGroundCrews);
                        let outcome = ground_crews::capture(
                            wings,
                            registered,
                            Buffer::Growing(&mut self.ground_demand),
                            Buffer::Growing(&mut self.ground_crews.rows),
                            |country, region| flags.get(country, region),
                        )
                        .unwrap_or_else(|reason| Outcome::Unavailable { reason });
                        if let Outcome::Rows { len } = outcome {
                            self.ground_crews.rows.truncate(len);
                        }
                        outcome
                    };
                    (outcome, elapsed(begin_ground))
                },
            );
        self.ground_crews.outcome = if stable || self.ground_lenient {
            ground_outcome
        } else {
            Outcome::Unavailable {
                reason: "Ground Crews writer certificate changed during acquisition",
            }
        };
        ticks[2] = ground_ticks;
        ticks[1] = elapsed(begin).saturating_sub(ground_ticks);
        ticks
    }

    pub fn fail_missions(
        &mut self,
        reason: &'static str,
        histories: &mut [air_histories::Staging],
    ) {
        self.fail_missions_rows(reason);
        self.missions = Some([0; 2]);
        air_histories::supply(
            histories,
            &collector_air_histories::history_changes::Unobserved,
            &mut air_histories::opening(),
        );
    }

    fn fail_missions_rows(&mut self, reason: &'static str) {
        self.registration_failure = Default::default();
        self.registrations.fail(failed(self.active[5], reason));
        self.patrol_combats.fail(failed(self.active[12], reason));
    }

    pub fn install_owned_mission_identities(&mut self, job: &mut AirWingJob) {
        let begin = qpc();
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
        self.identity_ticks = self.identity_ticks.saturating_add(elapsed(begin));
    }

    pub fn finish_wings(&mut self, source: Outcome) {
        if !self.identities_owned {
            self.identity_outcome = require_parent(source, self.identity_outcome);
        }
        self.equipment.outcome = require_parent(source, self.equipment.outcome);
        self.cumulative.outcome = if self.active[2] {
            require_parent(source, self.cumulative.outcome)
        } else {
            SKIPPED
        };
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        scores: Option<&mut engine_raid_inputs::raid_air_scores::Held>,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.bases.reset();
        self.carrier.reset();
        self.aces.reset();
        self.statistics.reset();
        if self.scores_early.is_none() {
            self.scores.reset();
            self.score_demand = [0; 2];
        }
        self.names.reset();
        let mut ticks = [0; N];
        if !self.bases_raw_mode {
            self.bases.outcome = BASES_NOT_STAGED;
        }
        if !self.active[3] {
            self.aces.outcome = SKIPPED;
        } else if !self.bases_raw_mode {
            self.aces.outcome = BASES_NOT_STAGED;
        }
        if !self.active[4] {
            self.statistics.outcome = SKIPPED;
        } else if !self.stats_raw_mode {
            self.statistics.outcome = STATISTICS_NOT_STAGED;
        }
        [ticks[5], ticks[12]] = self.missions.unwrap_or_default();
        let join_ticks = match self.joins {
            Some(ticks) => ticks,
            None => self.capture_joins(memory, base, game_state),
        };
        [ticks[6], ticks[7], ticks[14]] = join_ticks;
        ticks[15] = self.transfer_ticks;
        ticks[13] = match self.scores_early.take() {
            Some(ticks) => ticks,
            None => {
                let mut part = ScorePart::vacant();
                self.swap_scores(&mut part);
                let mut fresh = engine_raid_inputs::raid_air_scores::Held::default();
                let ticks = part.capture(
                    memory,
                    base,
                    game_state,
                    hour,
                    force_full,
                    self.score_capacity,
                    scores.unwrap_or(&mut fresh),
                );
                self.swap_scores(&mut part);
                ticks
            }
        };
        ticks
    }

    pub fn score_capacity(&self) -> [usize; 2] {
        self.score_capacity
    }

    fn swap_scores(&mut self, part: &mut ScorePart) {
        std::mem::swap(&mut self.scores, &mut part.rows);
        std::mem::swap(&mut self.score_demand, &mut part.demand);
    }

    pub fn install_scores(&mut self, part: &mut ScorePart) {
        let ticks = part.ticks;
        self.swap_scores(part);
        self.scores_early = Some(ticks);
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let sorties = self.sorties.outcomes();
        [
            self.equipment.outcome,
            self.bases.outcome,
            self.cumulative.outcome,
            self.aces.outcome,
            self.statistics.outcome,
            self.registrations.outcome,
            self.references.outcome,
            sorties[0],
            sorties[1],
            sorties[2],
            sorties[3],
            sorties[4],
            self.patrol_combats.outcome,
            self.scores.outcome,
            self.ground_crews.outcome,
            self.transfers.outcome,
        ]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("no projection supplied"),
        })
    }

    pub fn table_inputs(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        let mut inputs = self.sampled_tables().map(TableInput::Sampled);
        inputs[1] = self.base_input(projection);
        inputs
    }

    fn base_input(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> TableInput {
        collector_projection::projection::project_table::<tables::AirBases, _>(
            projection,
            self.bases.outcome,
            &self.bases.rows,
            |row| row.id as u128,
            |row, old| {
                row.id == old.id
                    && row.state == old.state
                    && row.province == old.province
                    && row.host == old.host
                    && row.capacity == old.capacity
                    && row.kind == old.kind
                    && self.carrier.equal(row.id, &old.carrier)
            },
            |row| rows::AirBaseRow {
                id: row.id,
                state: row.state,
                province: row.province,
                host: row.host,
                capacity: row.capacity,
                kind: row.kind,
                carrier: self.carrier.packet(row.id),
            },
        )
    }

    fn sampled_tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        let [missions, containers, modifiers, special_bases, metadata] = self.sorties.tables();
        [
            table::<tables::WingEquipment, _>(outcomes[0], &self.equipment.rows, Clone::clone),
            TableView::Unavailable {
                reason: String::new(),
            },
            table::<tables::WingCumulative, _>(outcomes[2], &self.cumulative.rows, |r| {
                rows::WingCumulativeRow {
                    id: r.id,
                    accidents: r.accidents,
                    experience: r.experience,
                }
            }),
            self.ace_table(outcomes[3]),
            table::<tables::AirStatistics, _>(outcomes[4], &self.statistics.rows, |r| {
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
                outcomes[6],
                &self.references.rows[..outcomes[6].len().min(self.references.rows.len())],
                match outcomes[1] {
                    Outcome::Rows { len } => self.bases.rows.get(..len),
                    _ => None,
                },
            ),
            missions,
            containers,
            modifiers,
            special_bases,
            metadata,
            table::<tables::AirNavalPatrolCombats, _>(
                outcomes[12],
                &self.patrol_combats.rows,
                Clone::clone,
            ),
            table::<tables::RaidAirScores, _>(outcomes[13], &self.scores.rows, |r| {
                rows::RaidAirScoreRow {
                    country: r.country,
                    region: r.region,
                    friendly: r.friendly,
                    enemy: r.enemy,
                    impact: rows::Recorded::Value(r.impact),
                    friendly_missions: r.friendly_missions,
                    enemy_missions: r.enemy_missions,
                    air_spotting_scale: r.air_spotting_scale,
                    menu: rows::Recorded::Value(r.menu.clone()),
                }
            }),
            table::<tables::AirGroundCrews, _>(outcomes[14], &self.ground_crews.rows, Clone::clone),
            table::<tables::AirTransferInputs, _>(outcomes[15], &self.transfers.rows, Clone::clone),
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
                active_order: self
                    .mission_order
                    .binary_search_by_key(&r.wing, |row| row.wing)
                    .ok()
                    .map(|at| self.mission_order[at].label),
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

fn merge_supply(
    ground: &mut Vec<rows::AirGroundCrewRow>,
    merged: &mut Vec<rows::AirGroundCrewRow>,
    supply: &[engine_map_air::map_air_supply::Summary],
    mut flag: impl FnMut(usize, usize) -> Option<u8>,
) -> Result<(), &'static str> {
    debug_assert!(ground.is_sorted_by_key(|row| (row.country, row.region)));
    debug_assert!(supply.is_sorted_by_key(|row| (row.country, row.region)));
    merged.clear();
    allocation::reserve(merged, ground.len().saturating_add(supply.len()))
        .map_err(|_| "regional Air supply projection allocation failed")?;
    let mut rows = ground.drain(..).peekable();
    for sample in supply {
        let key = (sample.country, sample.region);
        while let Some(row) = rows.next_if(|row| (row.country, row.region) < key) {
            merged.push(row);
        }
        let summary = Some(rows::AirSupplySummary {
            severity: sample.severity,
            needed: sample.needed,
            supplied: sample.supplied,
            deficit: sample.deficit,
        });
        match rows.next_if(|row| (row.country, row.region) == key) {
            Some(mut row) => {
                row.supply = rows::Recorded::Value(summary);
                merged.push(row);
            }
            None => merged.push(rows::AirGroundCrewRow {
                country: sample.country,
                region: sample.region,
                enabled: if sample.region == u32::MAX {
                    None
                } else {
                    flag(sample.country_pointer, sample.region_pointer).map(u32::from)
                },
                supply: rows::Recorded::Value(summary),
            }),
        }
    }
    merged.extend(rows);
    std::mem::swap(ground, merged);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[cfg(any(test, feature = "test-support"))]
pub fn capture_references(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    handles: &mut Vec<u64>,
    output: &mut Vec<BaseWitness>,
) -> Result<Outcome, &'static str> {
    capture_references_buffer(
        memory,
        base,
        game_state,
        wings,
        registered,
        Buffer::Growing(handles),
        Buffer::Growing(output),
        None,
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
    mut transfer_demand: Option<&mut Vec<usize>>,
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
    for wing in wings.iter().filter(|wing| wing.air_base != 0) {
        want(wing.air_base);
    }
    if let Some(demand) = transfer_demand.as_mut() {
        allocation::advisory(|| allocation::reserve(demand, wings.len()))
            .map_err(|_| "Air transfer demand storage unavailable")?;
        demand.clear();
    }
    for (index, wing) in wings
        .iter()
        .enumerate()
        .filter(|(_, wing)| wing.base_override != 0)
    {
        want(wing.base_override);
        if let Some(demand) = transfer_demand.as_mut() {
            demand.push(index);
        }
    }
    if required > handles.len() {
        return Ok(Outcome::Truncated {
            len: 0,
            unread_rows: required.min(u32::MAX as usize) as u32,
        });
    }
    let probes = &mut handles[..required];
    probes.sort_unstable();
    let (mut distinct, mut previous, mut next) = (0, 0, 0);
    while next < probes.len() {
        let handle = probes[next];
        next += 1;
        if handle != previous {
            previous = handle;
            probes[distinct] = handle;
            distinct += 1;
        }
    }
    let mut len = 0usize;
    let mut unread_rows = 0_u32;
    let mut pass = map_air_base_reference::Pass::new(base);
    let mut gathered = [None; map_air_base_reference::BASE_BATCH];
    for chunk in probes[..distinct].chunks(map_air_base_reference::BASE_BATCH) {
        let read = pass.read_bases(memory, game_state, chunk, &mut gathered);
        for (index, &handle) in chunk.iter().enumerate() {
            output.ensure(len.checked_add(1).ok_or("air reference count overflow")?)?;
            let reference = if index < read {
                gathered[index]
            } else {
                pass.read_base(memory, game_state, handle)?
            };
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
    }
    air_reference_witness::capture_hosts(&mut output[..len], |handle| {
        pass.read_host_owner(memory, handle)
    })?;
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

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
        engine_air_history::air_kamikaze::task_force_combats(memory, base, country, emit)
    })
}
#[cfg(any(test, feature = "test-support"))]
pub fn patrol_combats(
    scratch: &mut Vec<u64>,
    count: usize,
    combats: &mut Vec<rows::AirNavalPatrolCombatRow>,
    walk: impl FnMut(
        u32,
        &mut dyn FnMut(
            engine_air_history::air_kamikaze::TaskForceCombat,
        ) -> Result<(), &'static str>,
    ) -> Result<(), &'static str>,
) -> Result<(Outcome, (usize, usize)), &'static str> {
    patrol_combats_buffer(
        Buffer::Growing(scratch),
        count,
        Buffer::Growing(combats),
        walk,
    )
}

fn patrol_combats_buffer(
    mut scratch: Buffer<'_, u64>,
    count: usize,
    mut combats: Buffer<'_, rows::AirNavalPatrolCombatRow>,
    mut walk: impl FnMut(
        u32,
        &mut dyn FnMut(
            engine_air_history::air_kamikaze::TaskForceCombat,
        ) -> Result<(), &'static str>,
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

fn read_equipment_carried(
    memory: &impl Memory,
    base: usize,
    source: &WingSource<'_>,
    mut output: engine_native_memory::walk::Buffer<'_, rows::WingEquipmentRow>,
    start: usize,
    reuse: Option<&mut WingReuse<'_>>,
) -> (Outcome, usize) {
    let Some(reuse) = reuse else {
        return read_equipment_buffer(memory, base, source, output, start);
    };
    let plan = reuse.plan(source.pointer, source.row.id);
    if let Some(rows) = plan.carry_rows()
        && let Some(end) = start.checked_add(rows)
        && output.ensure(end) == Ok(true)
    {
        let carried = reuse.carry(&plan);
        output[start..end].clone_from_slice(carried);
        return (Outcome::Rows { len: rows }, rows);
    }
    let (outcome, required) = read_equipment_buffer(memory, base, source, output.suffix(0), start);
    let read = output.get(start..start + outcome.len()).unwrap_or_default();
    reuse.read(&plan, outcome, read);
    (outcome, required)
}

fn read_equipment_buffer(
    memory: &impl Memory,
    base: usize,
    source: &WingSource<'_>,
    mut output: engine_native_memory::walk::Buffer<'_, rows::WingEquipmentRow>,
    start: usize,
) -> (Outcome, usize) {
    let mut required = 0;
    let result = (|| {
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

#[derive(Debug, Default, Clone, Copy)]
pub struct WingMembershipRead {
    pub value: engine_native_air::cair_wing::WingMembership,
    pub error: Option<&'static str>,
}

mod ground_crews;
mod mission_store;
mod registration_reuse;
mod registration_store;
mod transfers;
mod wing;
mod wing_map_flags;
mod wing_raid_flags;
mod wing_store;
pub use mission_store::{MissionReuse, MissionStore};
#[cfg(any(test, feature = "test-support"))]
pub use registration_reuse::DecodedCounts;
pub use registration_reuse::DecodedRegistrations;
#[cfg(any(test, feature = "test-support"))]
pub use registration_store::Counts as RegistrationCounts;
pub use registration_store::RegistrationStore;
#[cfg(test)]
use wing::WING_PACK_N;
pub use wing::{
    AirWingCopyContext, AirWingCopyReport, AirWingJob, WING_PARTS, copy_air_refresh,
    decode_air_payload, part_starts, retained_store_bytes,
};
pub use wing_store::{WingReuse, WingStore};

mod stats;
#[cfg(test)]
use stats::{AIR_STATS_N, STATS_MASK, copy_stats_channels, sspan};
pub use stats::{
    AirStatsCopyContext, AirStatsCopyReport, AirStatsJob, copy_air_stats_refresh,
    decode_air_stats_payload,
};

pub mod bases;
pub mod carrier;
mod carrier_memory;
mod census_store;
mod state_fuel_inputs;
use super::fog;
pub use bases::{
    AIR_BASES_N, AirBasesCopyContext, AirBasesCopyReport, AirBasesJob, AirBasesMemo,
    AirBasesMemoContext, AirBasesMemoReport, HeldSegment,
    copy_air_bases_refresh,
};
#[cfg(any(test, feature = "test-support"))]
pub use bases::copy_air_bases_fixture;
#[cfg(test)]
use bases::{
    AIR_REGISTRATIONS_MASK, CarryRun, bases_pool_u32, bspan, copy_air_bases_pass, resolve_link,
};
#[cfg(test)]
pub(super) use state_fuel_inputs::Packet as StateFuelPacket;

#[cfg(test)]
mod census_store_tests;
#[cfg(test)]
mod registration_bench;
#[cfg(any(test, feature = "test-support"))]
pub mod registration_store_tests;

#[cfg(any(test, feature = "test-support"))]
mod tests;

#[cfg(test)]
mod transport_witness_tests;

#[cfg(test)]
mod fixed_mirror_gather_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod wing_test_kit;

#[cfg(any(test, feature = "test-support"))]
pub mod stats_test_kit;

#[cfg(test)]
mod air_bases_memo_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod air_bases_test_kit;

#[cfg(test)]
#[path = "air_mission_scale_tests.rs"]
mod mission_scale_tests;

#[cfg(test)]
#[path = "air/wing_store_tests.rs"]
mod wing_store_tests;

#[cfg(test)]
#[path = "air/mission_store_tests.rs"]
mod mission_store_tests;
