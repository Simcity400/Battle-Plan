//! Hourly task-force mission, link and supply operands, fleet colour, region count and ordered
//! roster, each task force's movement operands and its registered mission regions, all from
//! one fleet walk. Port searching/status inputs are independent optional groups;
//! raw strings use a separate loan of the existing Names arena type.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::allocation::{collect_vec, copy_vec, owned_vec};
use engine::{Memory, cfleet, ctask_force, cunit::FirstCombatMemo, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{
        FleetCounterInputRow, FreshTaskForceCounterEta, NavalEngagementTarget,
        NavalPortMissionInputs, NavalSearcher, Recorded, TableId, TaskForceCounterInputRow,
        TaskForceMapMotionRow, TaskForceMissionRow, tables,
    },
};

pub(crate) struct TransportCapture<'a> {
    pub guard: Result<(), &'static str>,
    pub receivers: &'a mut super::transport_receivers::Captured,
    pub shared: &'a mut super::transport_mission_shared::Shared,
}

struct RiskCapture<'a> {
    guard: Result<(), &'static str>,
    receivers: Option<&'a mut super::transport_receivers::Captured>,
    ships: Option<&'a mut dyn ctask_force::OriginalRiskShips>,
}

/// F99BF0 cache pair from the admitted original mission control, one sixteen
/// byte observation. Earlier native skips never demand this independent source.
fn capture_receiver_cache(memory: &impl Memory, control: &mut super::transport_receivers::Control) {
    if control.packet.in_regions.is_some_and(|flag| flag != 0)
        && control
            .packet
            .combat_references
            .is_some_and(|count| count <= 0)
    {
        control.packet.detection_cache = control
            .pointer
            .checked_add(ctask_force::NAVAL_MISSION)
            .and_then(|root| root.checked_add(0x20))
            .and_then(|at| engine::read::<[u8; 16]>(memory, at))
            .map(|bytes| {
                [
                    i64::from_le_bytes(bytes[..8].try_into().unwrap()),
                    i64::from_le_bytes(bytes[8..].try_into().unwrap()),
                ]
            });
    }
}

fn project_receiver_risk(
    control: &mut super::transport_receivers::Control,
    original: Option<engine::ctask_force::MissionRiskInputs>,
) {
    let Some(original) = original else {
        control.packet.risk = None;
        return;
    };
    control.packet.combined_detection = original.combined_detection.ok().flatten();
    control.packet.risk =
        Some(replay::rows::TransportReceiverRisk {
            coordination: original.coordination.ok().flatten(),
            spotting_chance: original.spotting_chance.ok().flatten(),
            night_spotting_chance: original.night_spotting_chance.ok().flatten(),
            visibility: original.visibility.ok().flatten(),
            speed_base: original.speed_base.ok().flatten(),
            speed_modifier: original.speed_modifier.ok().flatten(),
            terrain_class: original.terrain_class.ok().flatten(),
            stats: original.stats.ok().flatten().map(|stats| {
                replay::rows::TransportTaskForceStats {
                    detection_138: stats.detection_138.ok(),
                    visibility_148: stats.visibility_148.ok(),
                    visibility_150: stats.visibility_150.ok(),
                    carrier_detection_160: stats.carrier_detection_160.ok(),
                }
            }),
            roster_owner: original.roster_owner.ok().flatten(),
            location: original.location.ok().flatten(),
            region: original.region.ok().flatten(),
            location_controller: original.location_controller.ok().flatten(),
            detached_activity: original.detached_activity.ok().flatten(),
            ship_count: original.ship_count.ok().flatten(),
            targeted_spotting_modifier: original.targeted_spotting_modifier.ok().flatten(),
            actual_terrain_speed: original.actual_terrain_speed.ok().flatten(),
        });
}

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::TaskForceCounterInputs,
    TableId::FleetCounterInputs,
    TableId::TaskForceMapMotion,
    TableId::TaskForceMissions,
];
/// One country/fleet/task-force traversal serves all four tables; its time is charged to task
/// forces.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "fleet and task-force counter, motion and mission operands",
    0,
    &[0, 1, 2, 3],
)];

static ARROW_REFUSALS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];

static FRESH_REFUSALS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];

pub(crate) fn fresh_eta_data() -> impl serde::Serialize {
    crate::status::status!({
        "base_speed_refused": FRESH_REFUSALS[0].load(std::sync::atomic::Ordering::Relaxed),
        "speed_modifier_refused": FRESH_REFUSALS[1].load(std::sync::atomic::Ordering::Relaxed),
        "terrain_class_refused": FRESH_REFUSALS[2].load(std::sync::atomic::Ordering::Relaxed),
        "modified_range_refused": FRESH_REFUSALS[3].load(std::sync::atomic::Ordering::Relaxed),
        "exiled_refused": FRESH_REFUSALS[4].load(std::sync::atomic::Ordering::Relaxed),
        "strategic_redeployment_refused": FRESH_REFUSALS[5].load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative component refusals in current-hour task-force counter ETA demand; old motion remains available",
    })
}

pub(crate) fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "movement_numerator_refused": ARROW_REFUSALS[0].load(std::sync::atomic::Ordering::Relaxed),
        "retreat_refused": ARROW_REFUSALS[1].load(std::sync::atomic::Ordering::Relaxed),
        "withdrawal_refused": ARROW_REFUSALS[2].load(std::sync::atomic::Ordering::Relaxed),
        "naval_transfer_refused": ARROW_REFUSALS[3].load(std::sync::atomic::Ordering::Relaxed),
        "combat_links_refused": ARROW_REFUSALS[4].load(std::sync::atomic::Ordering::Relaxed),
        "cached_name_refused": ARROW_REFUSALS[5].load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative field-local task-force movement, combat-link and cached-name refusals; established counter and motion fields remain available",
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    task_forces: usize,
    fleets: usize,
    rosters: usize,
    motion: usize,
    paths: usize,
    missions: usize,
    regions: usize,
    names: usize,
    supply_origins: usize,
    port_searchers: usize,
    port_avoided: usize,
    port_bytes: usize,
    port_answers: usize,
}

impl Capacities {
    /// Native fleet/task rosters (Serialize@140D4A390) and both CUnit routes
    /// (Serialize@140BF7010) size every independent motion/counter arena.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let countries = engine::ccountry::Reader::new(memory, base, state)?;
        let mut demand = *self;
        let mut fleets = 0_usize;
        let mut tasks = 0_usize;
        let mut paths = 0_usize;
        let mut regions = 0_usize;
        let mut names = 0_usize;
        let mut origins = 0_usize;
        let mut port = ctask_force::port_mission::Demand::default();
        let mut refusal = None;
        let add = |total: &mut usize, count: usize| {
            *total = total
                .checked_add(count)
                .ok_or("naval counter sizing overflow")?;
            Ok::<(), &'static str>(())
        };
        cfleet::visit_task_forces(memory, base, &countries, |visit| {
            match visit {
                cfleet::Visit::Fleet { .. } => add(&mut fleets, 1)?,
                cfleet::Visit::TaskForce(task, _) => {
                    add(&mut tasks, 1)?;
                    for offset in [
                        engine::cunit::ORDINARY_ROUTE_DATA,
                        engine::cunit::ALTERNATE_ROUTE_DATA,
                    ] {
                        let array = task
                            .task
                            .checked_add(offset)
                            .and_then(|at| engine::cpdx_array::Array::read(memory, at, 4))
                            .ok_or("task-force route sizing header unreadable or invalid")?;
                        add(&mut paths, array.count as usize)?;
                    }
                    let array = task
                        .task
                        .checked_add(ctask_force::NAVAL_MISSION)
                        .and_then(|root| {
                            root.checked_add(engine::cnaval_mission::ACCESSIBLE_REGIONS)
                        })
                        .and_then(|at| engine::cpdx_array::Array::read(memory, at, 8))
                        .ok_or("task-force region sizing header unreadable or invalid")?;
                    add(&mut regions, array.count as usize)?;
                    let observed =
                        ctask_force::port_mission::demand(memory, base, task.task, task.kind);
                    add(&mut port.searchers, observed.searchers)?;
                    add(&mut port.avoided, observed.avoided)?;
                    add(&mut port.bytes, observed.bytes)?;
                    add(&mut port.references, observed.references)?;
                    if let Some(reason) = observed.refusal {
                        refusal.get_or_insert(reason);
                    }
                    for result in [
                        ctask_force::cached_name_demand(memory, base, task.task, task.id)
                            .and_then(|count| add(&mut names, count)),
                        engine::cunit_supply_inputs::origin_demand(memory, task.task)
                            .map_err(|failure| failure.reason())
                            .and_then(|count| add(&mut origins, count)),
                    ] {
                        if let Err(reason) = result {
                            refusal.get_or_insert(reason);
                        }
                    }
                }
            }
            Ok(())
        })?;
        demand.task_forces = demand.task_forces.max(tasks);
        demand.fleets = demand.fleets.max(fleets);
        demand.rosters = demand.rosters.max(tasks);
        demand.motion = demand.motion.max(tasks);
        demand.paths = demand.paths.max(paths);
        demand.missions = demand.missions.max(tasks);
        demand.regions = demand.regions.max(regions);
        demand.names = demand.names.max(names);
        demand.supply_origins = demand.supply_origins.max(origins);
        demand.raise_for_port(port)?;
        *self = demand;
        refusal.map_or(Ok(()), Err)
    }

    pub(crate) fn raise_for_names(&mut self, rows: usize, bytes: usize) {
        self.task_forces = self.task_forces.max(rows);
        self.names = self.names.max(bytes);
    }

    pub(crate) fn raise_for_supply_origins(&mut self, origins: usize) {
        self.supply_origins = self.supply_origins.max(origins);
    }

    pub(crate) fn raise_for_port(
        &mut self,
        demand: engine::ctask_force::port_mission::Demand,
    ) -> Result<(), &'static str> {
        self.port_searchers = self.port_searchers.max(demand.searchers);
        self.port_avoided = self.port_avoided.max(demand.avoided);
        self.port_bytes = self.port_bytes.max(demand.bytes);
        let answers = engine::ctask_force::port_mission::reference_slots(demand.references)
            .ok_or("port reference sizing overflow")?;
        self.port_answers = self.port_answers.max(answers);
        Ok(())
    }
}

pub struct Captured {
    task_forces: CaptureRows<ctask_force::CounterRow>,
    fleets: CaptureRows<cfleet::CounterRow>,
    rosters: Arena<u64>,
    motion: CaptureRows<ctask_force::MapMotionRow>,
    paths: Arena<u32>,
    /// First-combat answers for this capture's task forces. The fleet walk runs apart from the
    /// division and railway-gun captures, so it keeps its own memo, sized off-thread.
    combats: FirstCombatMemo,
    missions: CaptureRows<ctask_force::MissionRow>,
    regions: Arena<u32>,
    names: Arena<u8>,
    supply_origins: Arena<engine::cunit_supply_inputs::Origin>,
    port_searchers: Arena<ctask_force::port_mission::Searcher>,
    port_avoided: Arena<Option<engine::cname_group_member::Span>>,
    /// A separate instance of the existing Names arena: port growth and
    /// faults never consume the earlier cached-name field's capacity.
    port_bytes: Arena<u8>,
    /// Reader Drop erases transient registry pointers before acquisition exits.
    port_answers: Arena<ctask_force::port_mission::ReferenceAnswer>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            task_forces: CaptureRows::new(capacity.task_forces),
            fleets: CaptureRows::new(capacity.fleets),
            rosters: Arena::new(capacity.rosters),
            motion: CaptureRows::new(capacity.motion),
            paths: Arena::new(capacity.paths),
            combats: FirstCombatMemo::try_new(capacity.motion)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
            missions: CaptureRows::new(capacity.missions),
            regions: Arena::new(capacity.regions),
            names: Arena::new(capacity.names),
            supply_origins: Arena::new(capacity.supply_origins),
            port_searchers: Arena::new(capacity.port_searchers),
            port_avoided: Arena::new(capacity.port_avoided),
            port_bytes: Arena::new(capacity.port_bytes),
            port_answers: Arena::new(capacity.port_answers),
        }
    }
    pub fn reset(&mut self) {
        self.clear_fresh_owner_demands();
        self.task_forces.reset();
        self.fleets.reset();
        self.rosters.reset();
        self.motion.reset();
        self.paths.reset();
        self.combats.clear();
        self.missions.reset();
        self.regions.reset();
        self.names.reset();
        self.supply_origins.reset();
        self.port_searchers.reset();
        self.port_avoided.reset();
        self.port_bytes.reset();
        self.port_answers.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.task_forces.allocated_bytes()
            + self.fleets.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.motion.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.combats.allocated_bytes()
            + self.missions.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.names.allocated_bytes()
            + self.supply_origins.allocated_bytes()
            + self.port_searchers.allocated_bytes()
            + self.port_avoided.allocated_bytes()
            + self.port_bytes.allocated_bytes()
            + self.port_answers.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.task_forces.grow(&mut capacity.task_forces);
        self.fleets.grow(&mut capacity.fleets);
        grow_capacity(&mut capacity.rosters, self.rosters.required);
        self.rosters.grow(&mut capacity.rosters);
        self.motion.grow(&mut capacity.motion);
        grow_capacity(&mut capacity.paths, self.paths.required);
        self.paths.grow(&mut capacity.paths);
        if self
            .combats
            .reserve(capacity.motion)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        self.missions.grow(&mut capacity.missions);
        grow_capacity(&mut capacity.regions, self.regions.required);
        self.regions.grow(&mut capacity.regions);
        self.names.grow(&mut capacity.names);
        self.supply_origins.grow(&mut capacity.supply_origins);
        self.port_searchers.grow(&mut capacity.port_searchers);
        self.port_avoided.grow(&mut capacity.port_avoided);
        self.port_bytes.grow(&mut capacity.port_bytes);
        self.port_answers.grow(&mut capacity.port_answers);
    }
    #[cfg(test)]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.capture_at(
            memory,
            base,
            state,
            None,
            Err("fresh task-force modifier layout unverified"),
        )
    }

    pub fn capture_with_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        modifier_guard: Result<(), &'static str>,
    ) -> [u64; N] {
        self.capture_at(memory, base, state, Some(hour), modifier_guard)
    }

    fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: Option<i32>,
        modifier_guard: Result<(), &'static str>,
    ) -> [u64; N] {
        self.capture_impl(
            memory,
            base,
            state,
            hour,
            modifier_guard,
            RiskCapture {
                guard: modifier_guard,
                receivers: None,
                ships: None,
            },
        )
    }

    pub(crate) fn capture_with_transport(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        modifier_guard: Result<(), &'static str>,
        transport: TransportCapture<'_>,
    ) -> [u64; N] {
        let TransportCapture {
            guard: risk_guard,
            receivers,
            shared,
        } = transport;
        let ticks = {
            let mut observer = shared.risk_ships_mut().observer(memory, base, hour).ok();
            self.capture_impl(
                memory,
                base,
                state,
                Some(hour),
                modifier_guard,
                RiskCapture {
                    guard: risk_guard,
                    receivers: Some(&mut *receivers),
                    ships: observer
                        .as_mut()
                        .map(|observer| observer as &mut dyn ctask_force::OriginalRiskShips),
                },
            )
        };
        for control in &receivers.controls.first[..receivers.controls.len] {
            shared.receiver(control.packet);
        }
        receivers.retire();
        shared.finish();
        ticks
    }

    fn capture_impl(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: Option<i32>,
        modifier_guard: Result<(), &'static str>,
        risk: RiskCapture<'_>,
    ) -> [u64; N] {
        let RiskCapture {
            guard: risk_guard,
            mut receivers,
            ships: risk_ships,
        } = risk;
        self.clear_fresh_owner_demands();
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        // Answers are valid for one capture only; clearing keeps the reserved room.
        self.combats.clear();
        let mut motion = ctask_force::MapMotionCollector::new_growing(
            memory,
            base,
            state,
            &mut self.motion.rows,
            &mut self.paths.first,
            &mut self.combats,
        )
        .with_fresh_modifier_guard(modifier_guard);
        if let Some(hour) = hour {
            motion = motion.with_fresh_hour(hour);
        }
        if let Some(observer) = risk_ships {
            motion = motion.with_risk_ship_observer(observer);
        }
        let mut missions = ctask_force::MissionCollector::new_growing(
            memory,
            base,
            state,
            &mut self.missions.rows,
            &mut self.regions.first,
        );
        let mut names = engine::cname_group_member::NameArena {
            bytes: engine::walk::Buffer::Growing(&mut self.names.first),
            len: 0,
            required: 0,
        };
        let mut supply = ctask_force::SupplyOrigins {
            origins: engine::walk::Buffer::Growing(&mut self.supply_origins.first),
            len: 0,
            required: 0,
        };
        let mut port = hour.map(|hour| {
            ctask_force::port_mission::Collector::new_growing(
                memory,
                base,
                hour,
                &mut self.port_searchers.first,
                &mut self.port_avoided.first,
                &mut self.port_bytes.first,
                &mut self.port_answers.first,
            )
        });
        let captured = cfleet::walk_counter_inputs_with_port_observed_growing(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.rosters.first,
            (
                &mut self.task_forces.rows,
                Some(&mut names),
                Some(&mut supply),
                port.as_mut(),
            ),
            |pointer, id| missions.visit(pointer, id),
            |pointer, id, original| {
                let joined = receivers.as_deref_mut().map_or(Ok(false), |receivers| {
                    receivers.supply_counter(memory, pointer, id, original)
                });
                if joined == Ok(true) {
                    let mut count = None;
                    let mut owner = None;
                    let risk = motion.visit_with_original_counter_contribution_observed(
                        pointer,
                        id,
                        match original {
                            Some(original) => original.map(|row| row.is_in_regions),
                            None => receivers
                                .as_deref()
                                .and_then(|receivers| {
                                    receivers.controls.first[..receivers.controls.len]
                                        .binary_search_by_key(&pointer, |control| control.pointer)
                                        .ok()
                                        .and_then(|index| {
                                            receivers.controls.first[index].packet.in_regions
                                        })
                                })
                                .ok_or(
                                    "transport unattempted counter in-regions acquisition refused",
                                ),
                        },
                        true,
                        risk_guard,
                        &mut count,
                        &mut owner,
                    );
                    if let Some(receivers) = receivers.as_deref_mut() {
                        let index = receivers.controls.first[..receivers.controls.len]
                            .binary_search_by_key(&pointer, |control| control.pointer)
                            .expect("joined original receiver");
                        let control = &mut receivers.controls.first[index];
                        control.packet.combat_references = count.and_then(Result::ok);
                        control.packet.logical_owner = owner
                            .unwrap_or_else(|| {
                                pointer
                                    .checked_add(engine::cunit::LOGICAL_OWNER)
                                    .and_then(|at| engine::read(memory, at))
                                    .ok_or("transport receiver logical owner unreadable")
                            })
                            .ok();
                        project_receiver_risk(control, risk.ok());
                        capture_receiver_cache(memory, control);
                    }
                } else {
                    if joined.is_err()
                        && let Some(receivers) = receivers.as_deref_mut()
                    {
                        receivers.refuse_join(pointer);
                    }
                    motion.visit(pointer, id);
                }
            },
        );
        // Genuine outside-census receivers borrow this SAME current-pass graph;
        // no ordinary motion row, route or census membership is manufactured.
        if let Some(receivers) = receivers {
            for control in &mut receivers.controls.first[..receivers.controls.len] {
                if control.counter_attempted {
                    continue;
                }
                let pointer = control.pointer;
                let in_regions = pointer
                    .checked_add(ctask_force::NAVAL_MISSION)
                    .and_then(|root| root.checked_add(0x30))
                    .and_then(|at| engine::read::<u8>(memory, at))
                    .ok_or("transport outside receiver in-regions unreadable");
                control.packet.in_regions = in_regions.ok();
                let count = if in_regions.is_ok_and(|flag| flag != 0) {
                    pointer
                        .checked_add(engine::cunit::COMBAT_REFERENCE_COUNT)
                        .and_then(|at| engine::read::<i32>(memory, at))
                        .ok_or("transport outside receiver combat count unreadable")
                } else {
                    Err("transport outside receiver combat count unreached")
                };
                control.packet.combat_references = count.ok();
                control.packet.logical_owner = pointer
                    .checked_add(engine::cunit::LOGICAL_OWNER)
                    .and_then(|at| engine::read(memory, at));
                control.packet.received_supply_ratio = pointer
                    .checked_add(engine::cunit::RECEIVED_SUPPLY_RATIO)
                    .and_then(|at| engine::read(memory, at));
                let demand = engine::transport_ledger_observation::risk_demand(
                    Ok(true),
                    Ok(true),
                    in_regions,
                    count,
                );
                let risk = motion.borrow_mission_contribution(
                    Ok((pointer, control.packet.task_force)),
                    demand,
                    true,
                    risk_guard,
                );
                project_receiver_risk(control, risk.ok());
                capture_receiver_cache(memory, control);
            }
        }

        let port = port
            .map(ctask_force::port_mission::Collector::finish)
            .unwrap_or_default();
        self.port_searchers.len = port.searching_len;
        self.port_searchers.required = port.searching_required;
        self.port_avoided.len = port.avoided_len;
        self.port_avoided.required = port.avoided_required;
        self.port_bytes.len = port.bytes_len;
        self.port_bytes.required = port.bytes_required;
        self.port_answers.required =
            ctask_force::port_mission::reference_slots(port.reference_required)
                .unwrap_or(usize::MAX);
        self.names.len = names.len;
        self.names.required = names.required;
        self.supply_origins.len = supply.len;
        self.supply_origins.required = supply.required;
        // A walk that fails leaves the motion and mission censuses unknown too.
        let failed = match captured.task_forces {
            Outcome::Unavailable { reason } => Some(reason),
            _ => None,
        };
        let walked = motion.finish(failed);
        let registered = missions.finish(failed);
        self.missions.outcome = registered.outcome;
        self.regions.len = registered.regions_len;
        self.regions.required = registered.required_regions;
        self.task_forces.outcome = captured.task_forces;
        let mut arrow_refusals = [0_u64; 6];
        let mut fresh_refusals = [0_u64; 6];
        for row in &self.task_forces.rows[..self.task_forces.outcome.len()] {
            if row.name.is_none() {
                arrow_refusals[5] += 1;
            }
        }
        self.fleets.outcome = captured.fleets;
        self.rosters.len = captured.roster_len;
        self.rosters.required = captured.roster_required;
        self.motion.outcome = walked.outcome;
        for row in &self.motion.rows[..walked.outcome.len()] {
            if let Some(fresh) = row.fresh {
                for (index, refused) in [
                    fresh.base_speed.is_err(),
                    fresh.speed_modifier.is_err(),
                    fresh.terrain_class.is_err(),
                    fresh.modified_range.is_err(),
                    fresh.exiled.is_err(),
                    fresh.strategic_redeployment.is_err(),
                ]
                .into_iter()
                .enumerate()
                {
                    if refused {
                        fresh_refusals[index] += 1;
                    }
                }
            }
            for (index, missing) in [
                row.movement_numerator.is_none(),
                row.retreat.is_none(),
                row.withdrawal.is_none(),
                row.naval_transfer.is_none(),
                row.combat_links.is_none(),
            ]
            .into_iter()
            .enumerate()
            {
                if missing {
                    arrow_refusals[index] += 1;
                }
            }
        }
        for (counter, refused) in FRESH_REFUSALS.iter().zip(fresh_refusals) {
            if refused != 0 {
                counter.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            }
        }
        for (counter, refused) in ARROW_REFUSALS.iter().zip(arrow_refusals) {
            if refused != 0 {
                counter.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.paths.len = walked.paths_len;
        self.paths.required = walked.required_paths;
        [qpc().saturating_sub(begin).max(0) as u64, 0, 0, 0]
    }
    fn supply_origins(
        &self,
        row: &ctask_force::CounterRow,
    ) -> Option<&[engine::cunit_supply_inputs::Origin]> {
        let len = row.supply?.origins_len.ok()?;
        let end = row.supply_origins_start.checked_add(len)?;
        self.supply_origins
            .first
            .get(..self.supply_origins.len)?
            .get(row.supply_origins_start..end)
    }

    /// Called off-thread. No native pointer or string descriptor is retained;
    /// nullable slots preserve independent faults and source ordering.
    fn port_inputs(&self, row: &ctask_force::CounterRow) -> Option<NavalPortMissionInputs> {
        let input = row.port_mission?;
        let span = |(start, len): engine::cname_group_member::Span| {
            start.checked_add(len).map(|end| start..end)
        };
        let searching = input.searching.and_then(|range| {
            let records = self
                .port_searchers
                .first
                .get(..self.port_searchers.len)?
                .get(span(range)?)?;
            Some(owned_vec(collect_vec(records.iter().map(|entry| {
                NavalSearcher {
                    reference: entry.reference,
                    resolves: entry.resolves,
                    roster_owner: entry.roster_owner,
                }
            }))))
        });
        let avoided_ship_types = input.avoided_ship_types.and_then(|range| {
            let records = self
                .port_avoided
                .first
                .get(..self.port_avoided.len)?
                .get(span(range)?)?;
            Some(owned_vec(collect_vec(records.iter().map(|entry| {
                let bytes = self
                    .port_bytes
                    .first
                    .get(..self.port_bytes.len)?
                    .get(span((*entry)?)?)?;
                Some(owned_vec(copy_vec(bytes)))
            }))))
        });
        Some(NavalPortMissionInputs {
            sample_hour: input.sample_hour,
            searching,
            engagement_target: input.engagement_target.map(|target| NavalEngagementTarget {
                reference: target.reference,
                resolves: target.resolves,
            }),
            avoided_ship_types,
        })
    }

    pub(super) fn observe_supply(&self, work: &mut super::supply_inputs::Work) {
        for row in self
            .task_forces
            .rows
            .iter()
            .take(self.task_forces.outcome.len())
        {
            work.observe(row.supply, row.supply_failure);
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.task_forces.outcome,
            self.fleets.outcome,
            self.motion.outcome,
            self.missions.outcome,
        ]
    }

    /// Same-pass authenticated roster owners of admitted moving task forces.
    /// The caller also requires a complete motion outcome before certifying its
    /// domain; failures and truncation cannot stand for an empty owner set.
    pub(crate) fn fresh_owner_demands(
        &self,
    ) -> impl Iterator<Item = (u32, Result<engine::ccountry::Country, &'static str>)> + '_ {
        self.motion.rows[..self.motion.outcome.len().min(self.motion.rows.len())]
            .iter()
            .filter_map(|row| row.fresh_owner.map(|owner| (row.logical_owner, owner)))
    }

    /// End the transient owner loan before this pack leaves Ordinary, on every
    /// success and failure path. These pointers never reach projection.
    pub(crate) fn clear_fresh_owner_demands(&mut self) {
        for row in &mut self.motion.rows {
            row.fresh_owner = None;
        }
    }

    pub fn tables(&self) -> [TableView; N] {
        let [tasks, fleets, ..] = self.outcomes();
        let task_table = || {
            table::<tables::TaskForceCounterInputs, _>(tasks, &self.task_forces.rows, |row| {
                task(
                    row,
                    self.names.first.get(..self.names.len),
                    self.supply_origins(row),
                    self.port_inputs(row),
                )
            })
        };
        let motion = self.motion_table();
        let missions = self.mission_table();
        let Some(rosters) = self.rosters.first.get(..self.rosters.len) else {
            return [
                task_table(),
                unavailable("fleet roster arena length invalid"),
                motion,
                missions,
            ];
        };
        let fleet_rows = &self.fleets.rows[..fleets.len().min(self.fleets.rows.len())];
        if fleet_rows.iter().any(|r| {
            r.start
                .checked_add(r.len)
                .is_none_or(|end| end > rosters.len())
        }) {
            return [
                task_table(),
                unavailable("fleet roster lies outside captured arena"),
                motion,
                missions,
            ];
        }
        [
            task_table(),
            table::<tables::FleetCounterInputs, _>(fleets, &self.fleets.rows, |r| {
                FleetCounterInputRow {
                    fleet: r.id,
                    red: r.rgba[0],
                    green: r.rgba[1],
                    blue: r.rgba[2],
                    alpha: r.rgba[3],
                    mission_region_count: r.mission_region_count,
                    task_forces: owned_vec(copy_vec(&rosters[r.start..r.start + r.len])),
                }
            }),
            motion,
            missions,
        ]
    }

    fn mission_table(&self) -> TableView {
        let Some(regions) = self.regions.first.get(..self.regions.len) else {
            return unavailable("task-force mission region arena length invalid");
        };
        let rows = &self.missions.rows[..self.missions.outcome.len().min(self.missions.rows.len())];
        if rows.iter().any(|row| {
            row.regions_start
                .checked_add(row.regions_len)
                .is_none_or(|end| end > regions.len())
        }) {
            return unavailable("task-force mission regions lie outside the captured arena");
        }
        table::<tables::TaskForceMissions, _>(self.missions.outcome, &self.missions.rows, |r| {
            TaskForceMissionRow {
                task_force: r.id,
                owner: r.owner,
                current_region: r.current_region,
                regions: owned_vec(copy_vec(
                    &regions[r.regions_start..r.regions_start + r.regions_len],
                )),
            }
        })
    }

    fn motion_table(&self) -> TableView {
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return unavailable("task-force route arena length invalid");
        };
        let rows = &self.motion.rows[..self.motion.outcome.len().min(self.motion.rows.len())];
        if rows.iter().any(|row| {
            row.ordinary_start
                .checked_add(row.ordinary_len)
                .and_then(|end| end.checked_add(row.alternate_len))
                .is_none_or(|end| end > paths.len())
        }) {
            return unavailable("task-force route lies outside the captured arena");
        }
        // Each task force sits in one fleet roster; a repeated handle is a torn census. Checked
        // here, off the game thread.
        let Ok(mut handles) = collect_vec(rows.iter().map(|row| row.id)) else {
            return unavailable("collector allocation failed");
        };
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return unavailable("task force appears in more than one fleet roster");
        }
        table::<tables::TaskForceMapMotion, _>(self.motion.outcome, &self.motion.rows, |r| {
            let alternate = r.ordinary_start + r.ordinary_len;
            TaskForceMapMotionRow {
                task_force: r.id,
                target: r.target,
                combat_role: r.combat_role,
                path: owned_vec(copy_vec(&paths[r.ordinary_start..alternate])),
                alternate_path: owned_vec(copy_vec(&paths[alternate..alternate + r.alternate_len])),
                movement_numerator: Recorded::Value(r.movement_numerator),
                retreat: Recorded::Value(r.retreat),
                withdrawal: Recorded::Value(r.withdrawal),
                naval_transfer: Recorded::Value(r.naval_transfer),
                combat_links: Recorded::Value(r.combat_links),
                fresh: Recorded::Value(r.fresh.map(|fresh| FreshTaskForceCounterEta {
                    sample_hour: fresh.sample_hour,
                    base_speed: fresh.base_speed.ok(),
                    speed_modifier: fresh.speed_modifier.ok(),
                    terrain_class: fresh.terrain_class.ok(),
                    modified_range: fresh.modified_range.ok(),
                    exiled: fresh.exiled.ok(),
                    strategic_redeployment: fresh.strategic_redeployment.ok(),
                })),
            }
        })
    }
}

fn task(
    r: &ctask_force::CounterRow,
    names: Option<&[u8]>,
    origins: Option<&[engine::cunit_supply_inputs::Origin]>,
    port_mission: Option<NavalPortMissionInputs>,
) -> TaskForceCounterInputRow {
    TaskForceCounterInputRow {
        task_force: r.id,
        is_in_regions: r.is_in_regions,
        running_away: r.running_away,
        engagement_rule: r.engagement_rule,
        mission_region_count: r.mission_region_count,
        bombardment_region: r.bombardment_region,
        repair_parent: r.repair_parent,
        detached_activity: r.detached_activity,
        repair_last_mission: r.repair_last_mission,
        received_supply_ratio: r.received_supply_ratio,
        attached_headquarters: r.attached_headquarters,
        name: Recorded::Value(r.name.and_then(|(start, len)| {
            names?
                .get(start..start.checked_add(len)?)
                .map(|bytes| owned_vec(copy_vec(bytes)))
        })),
        supply: Recorded::Value(super::supply_inputs::project(r.supply, origins)),
        port_mission: Recorded::Value(port_mission),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    use replay::rows::TableRows;

    #[test]
    fn default_cold_storage_captures_original_headers_and_live_hints() {
        use engine::{ccountry, cgame_state, cnaval_mission, cstrategic_region, cunit};
        fn p32(bytes: &mut [u8], at: usize, value: u32) {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn p64(bytes: &mut [u8], at: usize, value: u64) {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
            p64(bytes, at, data as u64);
            p32(bytes, at + 8, count);
            p32(bytes, at + 12, count);
        }
        const STATE: usize = 0x1000;
        const COUNTRY: usize = 0x3000;
        const FLEET: usize = 0x4000;
        const TASK: usize = 0x5000;
        const REGION: usize = 0x8000;
        let mut bytes = vec![0; 0x9000];
        array(&mut bytes, STATE + cgame_state::COUNTRIES, 0x2000, 2);
        array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x2020, 2);
        array(&mut bytes, STATE + cgame_state::PROVINCES, 0x2050, 2);
        array(
            &mut bytes,
            STATE + cgame_state::STRATEGIC_REGIONS,
            0x2060,
            2,
        );
        p64(&mut bytes, 0x2008, COUNTRY as u64);
        p32(&mut bytes, 0x2024, 1);
        p64(&mut bytes, 0x2068, REGION as u64);
        p64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, COUNTRY + ccountry::ID, 1);
        array(&mut bytes, COUNTRY + ccountry::FLEETS, 0x2040, 1);
        p64(&mut bytes, 0x2040, FLEET as u64);
        p64(&mut bytes, FLEET, cfleet::VTABLE_RVA as u64);
        p64(&mut bytes, FLEET + cfleet::HANDLE, (1 << 32) | 61);
        p64(&mut bytes, FLEET + cfleet::OWNER, COUNTRY as u64);
        array(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x2030, 1);
        p64(&mut bytes, 0x2030, TASK as u64);
        p64(&mut bytes, TASK, ctask_force::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            TASK + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA as u64,
        );
        p64(&mut bytes, TASK + cunit::HANDLE, (1 << 32) | 51);
        p64(&mut bytes, TASK + ctask_force::FLEET, FLEET as u64);
        p32(&mut bytes, TASK + cunit::ROSTER_OWNER, 1);
        p32(&mut bytes, TASK + cunit::LOGICAL_OWNER, 1);
        p32(&mut bytes, TASK + ctask_force::MISSION, 2);
        let mission = TASK + ctask_force::NAVAL_MISSION;
        p64(&mut bytes, mission, cnaval_mission::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            mission + cnaval_mission::TASK_FORCE,
            TASK as u64,
        );
        p32(&mut bytes, mission + cnaval_mission::ENGAGEMENT_RULE, 2);
        array(
            &mut bytes,
            mission + cnaval_mission::ACCESSIBLE_REGIONS,
            0x7020,
            1,
        );
        p64(&mut bytes, 0x7020, REGION as u64);
        p64(&mut bytes, REGION, cstrategic_region::VTABLE_RVA as u64);
        p32(&mut bytes, REGION + cstrategic_region::ID, 1);
        for offset in [cunit::ORDINARY_ROUTE_DATA, cunit::ALTERNATE_ROUTE_DATA] {
            array(&mut bytes, TASK + offset, 0x7010, 1);
        }
        p32(&mut bytes, 0x7010, 1);
        let name = TASK + 0x258;
        bytes[name..name + 4].copy_from_slice(b"Navy");
        p64(&mut bytes, name + 16, 4);
        p64(&mut bytes, name + 24, 15);
        array(
            &mut bytes,
            TASK + engine::cunit_supply_inputs::ORIGINS,
            0x7100,
            1,
        );
        array(&mut bytes, TASK + 0x610, 0x7030, 1);
        array(&mut bytes, TASK + 0x740, 0x7200, 1);
        bytes[0x7200..0x7203].copy_from_slice(b"mod");
        p64(&mut bytes, 0x7210, 3);
        p64(&mut bytes, 0x7218, 15);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut capacity = Capacities::default();
        let mut captured = Captured::with_capacities(&capacity);
        assert_eq!(captured.allocated_bytes(), 0);
        captured.capture_with_hour(&memory, 0, STATE, 123, Err("fixture modifier refusal"));
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
        assert_eq!(
            (
                captured.rosters.len,
                captured.paths.len,
                captured.regions.len
            ),
            (1, 2, 1)
        );
        assert_eq!((captured.names.len, captured.supply_origins.len), (4, 1));
        assert_eq!(
            (
                captured.port_searchers.len,
                captured.port_avoided.len,
                captured.port_bytes.len
            ),
            (1, 1, 3)
        );
        assert_eq!(
            captured.task_forces.rows[0].supply.unwrap().origins_len,
            Ok(1)
        );
        assert!(
            captured
                .tables()
                .iter()
                .all(|table| matches!(table, TableView::Complete(_)))
        );
        capacity.presize(&memory, 0, STATE).unwrap();
        assert_eq!(
            (
                capacity.task_forces,
                capacity.fleets,
                capacity.rosters,
                capacity.motion,
                capacity.missions
            ),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(
            (
                capacity.paths,
                capacity.regions,
                capacity.names,
                capacity.supply_origins
            ),
            (2, 1, 4, 1)
        );
        assert_eq!(
            (
                capacity.port_searchers,
                capacity.port_avoided,
                capacity.port_bytes,
                capacity.port_answers
            ),
            (1, 1, 3, 4)
        );
    }

    #[test]
    fn transport_receiver_cache_keeps_one_observation_signed_lanes_and_native_gates() {
        use std::cell::RefCell;

        struct CacheMemory {
            address: usize,
            bytes: [u8; 16],
            fail: bool,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for CacheMemory {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                if self.fail || address != self.address || output.len() != self.bytes.len() {
                    return false;
                }
                output.copy_from_slice(&self.bytes);
                true
            }
        }

        let pointer = 0x1000;
        let address = pointer + ctask_force::NAVAL_MISSION + 0x20;
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&i64::MIN.to_le_bytes());
        bytes[8..].copy_from_slice(&i64::MAX.to_le_bytes());
        let mut memory = CacheMemory {
            address,
            bytes,
            fail: false,
            reads: RefCell::new(Vec::new()),
        };
        let mut control = super::super::transport_receivers::Control {
            pointer,
            packet: replay::rows::TransportMissionReceiver {
                in_regions: Some(255),
                combat_references: Some(-1),
                ..Default::default()
            },
            ..Default::default()
        };
        capture_receiver_cache(&memory, &mut control);
        assert_eq!(control.packet.detection_cache, Some([i64::MIN, i64::MAX]));
        assert_eq!(*memory.reads.borrow(), [(address, 16)]);

        memory.reads.borrow_mut().clear();
        memory.fail = true;
        control.packet.combat_references = Some(0);
        capture_receiver_cache(&memory, &mut control);
        assert_eq!(control.packet.detection_cache, None);
        assert_eq!(*memory.reads.borrow(), [(address, 16)]);

        memory.reads.borrow_mut().clear();
        for (in_regions, combat_references) in [
            (None, Some(0)),
            (Some(0), Some(0)),
            (Some(1), None),
            (Some(1), Some(1)),
        ] {
            control.packet.in_regions = in_regions;
            control.packet.combat_references = combat_references;
            capture_receiver_cache(&memory, &mut control);
            assert_eq!(control.packet.detection_cache, None);
        }
        assert!(memory.reads.borrow().is_empty());
    }

    #[test]
    fn unreadable_countries_fail_every_table_and_rosters_convert_in_native_order() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 2,
            motion: 1,
            paths: 1,
            missions: 1,
            regions: 1,
            names: 0,
            supply_origins: 0,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
        });
        capture.capture(&memory, 0, 0);
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.reset();
        capture.fleets.rows[0] = cfleet::CounterRow {
            id: (1 << 32) | 61,
            rgba: [0x3f80_0000, 0, 0x8000_0000, u32::MAX],
            mission_region_count: 2,
            start: 0,
            len: 2,
        };
        capture.rosters.first[..2].copy_from_slice(&[(54 << 32) | 51, (43 << 32) | 51]);
        capture.rosters.len = 2;
        capture.fleets.outcome = Outcome::Rows { len: 1 };
        capture.task_forces.outcome = Outcome::Rows { len: 0 };
        capture.motion.outcome = Outcome::Rows { len: 0 };
        let [
            _,
            TableView::Complete(TableRows::FleetCounterInputs(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].task_forces, [(54 << 32) | 51, (43 << 32) | 51]);
        assert_eq!(rows[0].blue, 0x8000_0000);
        capture.fleets.rows[0].len = 3;
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }

    #[test]
    fn cached_task_force_names_are_owned_and_failure_never_hides_counter_prefixes() {
        let mut capacity = Capacities {
            task_forces: 2,
            names: 7,
            ..Default::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: 51,
            received_supply_ratio: 35_000,
            name: Some((0, 0)),
            ..Default::default()
        };
        capture.task_forces.rows[1] = ctask_force::CounterRow {
            id: 61,
            running_away: 1,
            name: Some((0, 7)),
            ..Default::default()
        };
        capture.names.first[..7].copy_from_slice(b"TF\0tail");
        capture.names.len = 7;
        capture.task_forces.outcome = Outcome::Rows { len: 2 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(owned)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("complete counter prefix");
        };
        assert_eq!(owned[0].name, Recorded::Value(Some(Vec::new())));
        assert_eq!(owned[1].name, Recorded::Value(Some(b"TF\0tail".to_vec())));
        capture.names.first[..7].copy_from_slice(b"Renamed");
        let TableView::Complete(TableRows::TaskForceCounterInputs(renamed)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("complete counter prefix");
        };
        assert_eq!(renamed[1].name, Recorded::Value(Some(b"Renamed".to_vec())));
        assert_eq!(
            owned[1].name,
            Recorded::Value(Some(b"TF\0tail".to_vec())),
            "projection owns its bytes"
        );
        capture.task_forces.rows[0].name = None;
        capture.task_forces.rows[1].name = Some((usize::MAX, 7));
        let TableView::Complete(TableRows::TaskForceCounterInputs(refused)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("name refusals preserve the table");
        };
        assert!(refused.iter().all(|row| row.name == Recorded::Value(None)));
        assert_eq!(refused[0].received_supply_ratio, 35_000);
        assert_eq!(refused[1].running_away, 1);
        capture.names.required = 35;
        capture.grow(&mut capacity);
        assert!(capacity.names >= 35 && capture.names.first.len() >= 35);
        assert_eq!(capture.names.len, 7);
        capacity.raise_for_names(100, 200);
        assert_eq!((capacity.task_forces, capacity.names), (100, 200));
        capture.reset();
        assert_eq!((capture.names.len, capture.names.required), (0, 0));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }

    #[test]
    fn motion_routes_split_from_the_arena_and_torn_censuses_are_refused() {
        let mut capacity = Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 1,
            motion: 2,
            paths: 3,
            missions: 1,
            regions: 1,
            names: 0,
            supply_origins: 0,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.motion.rows[0] = ctask_force::MapMotionRow {
            id: (5 << 32) | 61,
            target: 9,
            combat_role: 2,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_len: 1,
            combat_links: Some([u64::MAX, 0x8000_0000_0000_0001]),
            ..Default::default()
        };
        capture.paths.first.copy_from_slice(&[7, 9, 11]);
        capture.paths.len = 3;
        capture.motion.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceMapMotion(rows)) = capture.motion_table()
        else {
            panic!("complete motion table");
        };
        assert_eq!(
            (rows[0].path.as_slice(), rows[0].alternate_path.as_slice()),
            (&[7, 9][..], &[11][..])
        );
        assert_eq!((rows[0].target, rows[0].combat_role), (9, 2));
        assert_eq!(
            rows[0].combat_links,
            Recorded::Value(Some([u64::MAX, 0x8000_0000_0000_0001]))
        );
        capture.motion.rows[0].combat_links = None;
        let TableView::Complete(TableRows::TaskForceMapMotion(refused)) = capture.motion_table()
        else {
            panic!("complete motion prefix after suffix refusal");
        };
        assert_eq!(refused[0].combat_links, Recorded::Value(None));
        assert_eq!((refused[0].target, refused[0].combat_role), (9, 2));
        capture.motion.rows[0].alternate_len = 2;
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
        capture.motion.rows[0].alternate_len = 0;
        capture.motion.rows[1] = capture.motion.rows[0];
        capture.motion.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
        capture.motion.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 3,
        };
        capture.paths.required = 12;
        capture.grow(&mut capacity);
        assert!(capacity.paths >= 12 && capture.paths.first.len() >= 12);
    }

    #[test]
    fn fresh_eta_projection_keeps_independent_refusals_and_clears_every_owner_loan() {
        let capacity = Capacities {
            motion: 2,
            ..Capacities::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        let owner = engine::ccountry::Country {
            pointer: 0x1234,
            id: 19,
            canonical_slot: 3,
        };
        capture.motion.rows[0] = ctask_force::MapMotionRow {
            id: 7,
            logical_owner: 22,
            fresh_owner: Some(Ok(owner)),
            fresh: Some(ctask_force::FreshEtaInputs {
                sample_hour: 123,
                base_speed: Err("fixture base refusal"),
                speed_modifier: Ok(i64::MIN),
                terrain_class: Ok(-1),
                modified_range: Ok(i64::MAX),
                exiled: Ok(0xfe),
                strategic_redeployment: Err("fixture redeployment refusal"),
            }),
            ..Default::default()
        };
        capture.motion.outcome = Outcome::Rows { len: 1 };
        assert_eq!(
            capture.fresh_owner_demands().collect::<Vec<_>>(),
            [(22, Ok(owner))]
        );
        let TableView::Complete(TableRows::TaskForceMapMotion(rows)) = capture.motion_table()
        else {
            panic!("complete prefix");
        };
        assert_eq!(
            rows[0].fresh,
            Recorded::Value(Some(FreshTaskForceCounterEta {
                sample_hour: 123,
                base_speed: None,
                speed_modifier: Some(i64::MIN),
                terrain_class: Some(-1),
                modified_range: Some(i64::MAX),
                exiled: Some(0xfe),
                strategic_redeployment: None,
            }))
        );
        // Even an unpublished staged prefix must release its native pointer.
        capture.motion.rows[1].fresh_owner = Some(Err("fixture owner refusal"));
        capture.motion.outcome = Outcome::Unavailable {
            reason: "fixture later census fault",
        };
        capture.clear_fresh_owner_demands();
        assert!(
            capture
                .motion
                .rows
                .iter()
                .all(|row| row.fresh_owner.is_none())
        );
        capture.motion.rows[1].fresh_owner = Some(Ok(owner));
        capture.reset();
        assert!(
            capture
                .motion
                .rows
                .iter()
                .all(|row| row.fresh_owner.is_none())
        );
        assert_eq!(capture.motion.rows[0].fresh.unwrap().sample_hour, 123);
        // Reset changes table availability; retained owned packet bytes are not
        // a fresh sample and cannot be projected through the unavailable prefix.
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn mission_regions_split_from_the_arena_and_stray_ranges_are_refused() {
        let mut capacity = Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 1,
            motion: 1,
            paths: 1,
            missions: 1,
            regions: 3,
            names: 0,
            supply_origins: 0,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.missions.rows[0] = ctask_force::MissionRow {
            id: (5 << 32) | 61,
            owner: 42,
            current_region: 2,
            regions_start: 0,
            regions_len: 3,
        };
        capture.regions.first.copy_from_slice(&[2, 1, 2]);
        capture.regions.len = 3;
        capture.missions.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceMissions(rows)) = capture.mission_table()
        else {
            panic!("complete mission table");
        };
        assert_eq!(
            (rows[0].task_force, rows[0].owner, rows[0].current_region),
            ((5 << 32) | 61, 42, 2)
        );
        assert_eq!(rows[0].regions, [2, 1, 2]);
        capture.missions.rows[0].regions_len = 4;
        assert!(matches!(
            capture.mission_table(),
            TableView::Unavailable { .. }
        ));
        capture.missions.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        capture.regions.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.regions >= 9 && capture.regions.first.len() >= 9);
    }

    #[test]
    fn supply_nodes_fail_independently_and_owned_ship_demand_keeps_names_and_prefixes() {
        use engine::cunit_supply_inputs::{Demand, Failure, Inputs, Origin};
        let mut capacity = Capacities {
            task_forces: 1,
            names: 4,
            supply_origins: 2,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
            ..Default::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.names.first[..4].copy_from_slice(b"Navy");
        capture.names.len = 4;
        capture.supply_origins.first[..2].copy_from_slice(&[
            Origin {
                province: 13,
                requested: -1,
                received: 2,
            },
            Origin {
                province: 13,
                requested: 3,
                received: -4,
            },
        ]);
        capture.supply_origins.len = 2;
        let input = Inputs {
            asked: 190_000,
            received: 23_000,
            ship_demand: Demand {
                count: 3,
                value: 190_000,
            },
            disruption: -9,
            origins_len: Ok(2),
            ..Default::default()
        };
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: (29 << 32) | 61,
            received_supply_ratio: 12_105,
            name: Some((0, 4)),
            supply: Some(input),
            ..Default::default()
        };
        capture.task_forces.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(owned)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("supply suffix must not hide the counter prefix")
        };
        let supply = owned[0]
            .supply
            .require("fixture supply")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(
            (supply.ship_demand.count, supply.ship_demand.value),
            (3, 190_000)
        );
        let origins = supply.origins.as_ref().unwrap();
        assert_eq!(
            (
                origins[1].province,
                origins[1].requested,
                origins[1].received
            ),
            (13, 3, -4)
        );
        capture.supply_origins.first[1].received = 999;
        assert_eq!(
            origins[1].received, -4,
            "projection owns the original ordered bytes"
        );
        for reason in [
            Failure::InvalidOrigins,
            Failure::OriginsChanged,
            Failure::OriginsCapacity { required: 41 },
        ] {
            capture.task_forces.rows[0].supply = Some(Inputs {
                origins_len: Err(reason),
                ..input
            });
            capture.task_forces.rows[0].supply_failure = Some(reason);
            let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
                capture.tables().into_iter().next().unwrap()
            else {
                panic!("origin refusal must preserve supply scalars")
            };
            let supply = rows[0]
                .supply
                .require("fixture supply")
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!(
                (supply.asked, supply.received, supply.disruption),
                (190_000, 23_000, -9)
            );
            assert_eq!(supply.ship_demand.count, 3);
            assert!(supply.origins.is_none());
            assert_eq!(rows[0].name, Recorded::Value(Some(b"Navy".to_vec())));
            assert_eq!(rows[0].received_supply_ratio, 12_105);
        }
        capture.supply_origins.required = 41;
        capture.grow(&mut capacity);
        assert!(capacity.supply_origins >= 41 && capture.supply_origins.first.len() >= 41);
        capture.task_forces.rows[0].supply = None;
        capture.task_forces.rows[0].supply_failure = Some(Failure::Unreadable);
        let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("scalar refusal must preserve the native counter prefix")
        };
        assert_eq!(rows[0].supply, Recorded::Value(None));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"Navy".to_vec())));
        assert_eq!(rows[0].received_supply_ratio, 12_105);
    }

    #[test]
    fn invalid_supply_origin_span_withholds_only_nodes_and_name_fault_keeps_supply() {
        use engine::cunit_supply_inputs::Inputs;
        let mut capture = Captured::with_capacities(&Capacities {
            task_forces: 1,
            ..Default::default()
        });
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: (29 << 32) | 61,
            running_away: 0xff,
            supply: Some(Inputs {
                received: i64::MIN,
                origins_len: Ok(1),
                ..Default::default()
            }),
            supply_origins_start: usize::MAX,
            ..Default::default()
        };
        capture.task_forces.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("an optional arena failure is not a counter census failure")
        };
        let supply = rows[0]
            .supply
            .require("fixture supply")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(supply.received, i64::MIN);
        assert!(supply.origins.is_none());
        assert_eq!(rows[0].name, Recorded::Value(None));
        assert_eq!(rows[0].running_away, 0xff);
    }

    #[test]
    fn port_projection_owns_ordered_fault_slots_and_exact_bytes_without_hiding_old_fields() {
        use ctask_force::port_mission::{EngagementTarget, Inputs, Searcher};
        let mut capture = Captured::with_capacities(&Capacities {
            task_forces: 1,
            names: 4,
            port_searchers: 3,
            port_avoided: 3,
            port_bytes: 3,
            ..Default::default()
        });
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: (29 << 32) | 61,
            received_supply_ratio: 12_345,
            name: Some((0, 4)),
            port_mission: Some(Inputs {
                sample_hour: 123,
                searching: Some((0, 3)),
                engagement_target: Some(EngagementTarget {
                    reference: (99 << 32) | 4713,
                    resolves: Some(false),
                }),
                avoided_ship_types: Some((0, 3)),
            }),
            ..Default::default()
        };
        capture.names.first[..4].copy_from_slice(b"Navy");
        capture.names.len = 4;
        capture.port_searchers.first.copy_from_slice(&[
            Searcher {
                reference: Some(0),
                resolves: Some(false),
                roster_owner: None,
            },
            Searcher {
                reference: Some((17 << 32) | 4713),
                resolves: Some(true),
                roster_owner: Some(0),
            },
            Searcher::default(),
        ]);
        capture.port_searchers.len = 3;
        capture
            .port_avoided
            .first
            .copy_from_slice(&[Some((0, 3)), None, Some((3, 0))]);
        capture.port_avoided.len = 3;
        capture.port_bytes.first.copy_from_slice(&[0xff, 0, 7]);
        capture.port_bytes.len = 3;
        capture.task_forces.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("optional port faults preserve the counter census")
        };
        let packet = rows[0]
            .port_mission
            .require("fixture port")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(packet.sample_hour, 123);
        assert_eq!(packet.searching.as_ref().unwrap()[1].roster_owner, Some(0));
        assert_eq!(packet.searching.as_ref().unwrap()[2].reference, None);
        assert_eq!(
            packet.engagement_target.as_ref().unwrap().reference,
            (99 << 32) | 4713
        );
        assert_eq!(
            packet.engagement_target.as_ref().unwrap().resolves,
            Some(false)
        );
        assert_eq!(
            packet.avoided_ship_types,
            Some(vec![Some(vec![0xff, 0, 7]), None, Some(vec![])])
        );
        capture.port_bytes.first.fill(0);
        capture.port_searchers.first.fill(Searcher::default());
        assert_eq!(
            packet.avoided_ship_types,
            Some(vec![Some(vec![0xff, 0, 7]), None, Some(vec![])])
        );
        assert_eq!(packet.searching.as_ref().unwrap()[1].roster_owner, Some(0));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"Navy".to_vec())));
        assert_eq!(rows[0].received_supply_ratio, 12_345);
        capture.task_forces.rows[0]
            .port_mission
            .as_mut()
            .unwrap()
            .searching = Some((usize::MAX, 3));
        capture.port_avoided.first[0] = Some((usize::MAX, 3));
        let packet = capture.port_inputs(&capture.task_forces.rows[0]).unwrap();
        assert!(packet.searching.is_none());
        assert_eq!(packet.avoided_ship_types.as_ref().unwrap()[0], None);
        assert_eq!(
            packet.engagement_target.as_ref().unwrap().resolves,
            Some(false)
        );
    }

    #[test]
    fn port_growth_and_sizing_never_change_cached_name_capacity_or_bytes() {
        let mut capacity = Capacities {
            names: 4,
            ..Default::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        let mut old_capacity = capacity;
        let mut old = Captured::with_capacities(&old_capacity);
        old.names.first[..4].copy_from_slice(b"Navy");
        old.names.len = 4;
        capture.names.first[..4].copy_from_slice(b"Navy");
        capture.names.len = 4;
        capture.port_searchers.required = 27;
        capture.port_avoided.required = 31;
        capture.port_bytes.required = 1025;
        capture.port_answers.required = 128;
        capture.grow(&mut capacity);
        old.grow(&mut old_capacity);
        assert_eq!(capacity.names, old_capacity.names);
        assert_eq!(capture.names.first.as_slice(), old.names.first.as_slice());
        assert_eq!(&capture.names.first[..capture.names.len], b"Navy");
        assert!(capacity.port_searchers >= 27 && capacity.port_avoided >= 31);
        assert!(capacity.port_bytes >= 1025 && capacity.port_answers >= 128);
        let before = capacity;
        capacity
            .raise_for_port(ctask_force::port_mission::Demand {
                searchers: 100,
                avoided: 200,
                bytes: 3000,
                references: 101,
                refusal: None,
            })
            .unwrap();
        assert_eq!(
            (
                capacity.port_searchers,
                capacity.port_avoided,
                capacity.port_bytes,
                capacity.port_answers
            ),
            (100, 200, 3000, 256)
        );
        assert_eq!(capacity.names, before.names);
        assert_eq!(capacity.supply_origins, before.supply_origins);
        capture.reset();
        assert_eq!(
            (
                capture.port_searchers.len,
                capture.port_searchers.required,
                capture.port_avoided.len,
                capture.port_avoided.required,
                capture.port_bytes.len,
                capture.port_bytes.required,
                capture.port_answers.required
            ),
            (0, 0, 0, 0, 0, 0, 0)
        );
    }
}
