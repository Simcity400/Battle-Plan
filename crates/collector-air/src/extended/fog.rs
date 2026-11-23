use collector_memory::read_accounting;
use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, bounded, grow_capacity},
};
use engine_allocation::{copy_vec, owned_vec, reserve, resize};
use engine_map_air::radar_group;
use engine_native::{Memory, ccontroller_area, ccountry, ctheatre, walk::Outcome};
use engine_native_air::cair_base;
use engine_visibility::unit_viewer_entry;
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableInput;

#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::TableView;

pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::RadarCoverage,
    TableId::VolunteerHosts,
    TableId::TheatreAreas,
    TableId::StationedUnits,
    TableId::UnitIntel,
    TableId::AirBaseRegistrations,
    TableId::SentVolunteers,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub radar_provinces: usize,
    pub area_provinces: usize,
    pub wings: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    radar_provinces,
    area_provinces,
    wings
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        use collector_extended_support::demand::rows;
        use collector_primitives::country_slots::raise;
        let mut failure = None;
        let mut accept = |capacity: &mut usize, outcome: Outcome| match rows(outcome) {
            Ok(count) => raise(capacity, count),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        };
        let radar = radar_group::count(memory, base, state);
        accept(&mut self.rows[0], radar.outcome);
        raise(&mut self.radar_provinces, radar.required_provinces);
        accept(
            &mut self.rows[1],
            ccountry::count_volunteer_hosts(memory, base, state),
        );
        accept(
            &mut self.rows[6],
            ccountry::count_sent_volunteers(memory, base, state),
        );
        let stationed = ctheatre::count_stationed(memory, base, state, true);
        accept(&mut self.rows[2], stationed.areas);
        accept(&mut self.rows[3], stationed.units);
        raise(&mut self.area_provinces, stationed.required_provinces);
        accept(
            &mut self.rows[4],
            unit_viewer_entry::count(memory, base, state),
        );
        let registrations = cair_base::walk_registrations(
            memory,
            base,
            state,
            cair_base::RegistrationBuffers {
                registrations: &mut [],
                wings: &mut [],
                resolved: &mut [],
            },
        );
        accept(&mut self.rows[5], registrations.outcome);
        raise(&mut self.wings, registrations.required_wings);
        failure.map_or(Ok(()), Err)
    }
}

pub struct Captured {
    radar: CaptureRows<radar_group::Row>,
    radar_provinces: Arena<u32>,
    volunteers: CaptureRows<ccountry::VolunteerHostRow>,
    sent_volunteers: CaptureRows<ccountry::SentVolunteerRow>,
    areas: CaptureRows<ccontroller_area::Row>,
    area_provinces: Arena<u32>,
    units: CaptureRows<ctheatre::StationedUnitRow>,
    intel: CaptureRows<unit_viewer_entry::Row>,
    registrations: CaptureRows<cair_base::RegistrationRow>,
    pub carrier: super::air::carrier::Registrations,
    pub carrier_scratch: super::air::carrier::Scratch,
    wings: Arena<u64>,
    resolved: Vec<cair_base::ResolvedLink>,
    required_links: usize,
    active: [bool; N],
    pub radar_raw_mode: bool,
    pub intel_raw_mode: bool,
    pub registrations_raw_mode: bool,
    pub stationed: bool,
}

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay_data::cadence::SKIPPED_REASON,
};

impl Captured {
    pub fn fail_gated_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        let outcomes = TABLES.map(|table| {
            collector_extended_support::outcomes::failed_capture(table, hour, force_full, reason)
        });
        if !self.radar_raw_mode {
            self.radar.fail(outcomes[0]);
        }
        self.volunteers.fail(outcomes[1]);
        self.sent_volunteers.fail(outcomes[6]);
        self.areas.fail(outcomes[2]);
        self.units.fail(outcomes[3]);
        if !self.intel_raw_mode {
            self.intel.fail(outcomes[4]);
        }
        if !self.registrations_raw_mode {
            self.registrations.fail(outcomes[5]);
        }
    }

    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            radar_provinces: 0,
            area_provinces: 0,
            wings: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            radar: CaptureRows::new(capacity.rows[0]),
            radar_provinces: Arena::new(capacity.radar_provinces),
            volunteers: CaptureRows::new(capacity.rows[1]),
            sent_volunteers: CaptureRows::new(capacity.rows[6]),
            areas: CaptureRows::new(capacity.rows[2]),
            area_provinces: Arena::new(capacity.area_provinces),
            units: CaptureRows::new(capacity.rows[3]),
            intel: CaptureRows::new(capacity.rows[4]),
            registrations: CaptureRows::new(capacity.rows[5]),
            carrier: super::air::carrier::Registrations::new(capacity.rows[5]),
            carrier_scratch: super::air::carrier::Scratch::new(),
            wings: Arena::new(capacity.wings),
            resolved: Vec::new(),
            required_links: 0,
            active: [true; N],
            radar_raw_mode: false,
            intel_raw_mode: false,
            registrations_raw_mode: false,
            stationed: true,
        }
    }

    pub fn reset(&mut self) {
        self.radar.reset();
        self.radar_provinces.reset();
        self.volunteers.reset();
        self.sent_volunteers.reset();
        self.areas.reset();
        self.area_provinces.reset();
        self.units.reset();
        self.intel.reset();
        self.registrations.reset();
        self.carrier.reset();
        self.wings.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.radar.allocated_bytes()
            + self.radar_provinces.allocated_bytes()
            + self.volunteers.allocated_bytes()
            + self.sent_volunteers.allocated_bytes()
            + self.areas.allocated_bytes()
            + self.area_provinces.allocated_bytes()
            + self.units.allocated_bytes()
            + self.intel.allocated_bytes()
            + self.registrations.allocated_bytes()
            + self.carrier.bytes()
            + self.carrier_scratch.bytes()
            + self.wings.allocated_bytes()
            + self.resolved.capacity() * size_of::<cair_base::ResolvedLink>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.radar.grow(&mut capacity.rows[0]);
        self.radar_provinces.grow(&mut capacity.radar_provinces);
        self.volunteers.grow(&mut capacity.rows[1]);
        self.sent_volunteers.grow(&mut capacity.rows[6]);
        self.areas.grow(&mut capacity.rows[2]);
        self.area_provinces.grow(&mut capacity.area_provinces);
        self.units.grow(&mut capacity.rows[3]);
        self.intel.grow(&mut capacity.rows[4]);
        grow_capacity(&mut capacity.rows[5], self.registrations.rows.len());
        grow_capacity(&mut capacity.wings, self.wings.first.len());
        self.registrations.grow(&mut capacity.rows[5]);
        self.carrier.grow();
        self.wings.grow(&mut capacity.wings);
        self.grow_resolver(self.required_links);
    }

    fn grow_resolver(&mut self, links: usize) {
        if links > self.resolved.len() {
            let _ = resize(&mut self.resolved, links, None).map_err(collector_memory::terminal::io);
        }
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active =
            TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_storage(memory, base, game_state, hour, force_full)
    }

    fn capture_storage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.reset();
        let mut ticks = [0; N];

        if !self.active[0] {
            self.radar.outcome = SKIPPED;
        } else if self.radar_raw_mode {
            ticks[0] = 0;
        } else {
            let _reads = read_accounting::table(TableId::RadarCoverage);
            let begin = qpc();
            let first = {
                radar_group::walk(
                    memory,
                    base,
                    game_state,
                    &mut self.radar.rows,
                    &mut self.radar_provinces.first,
                )
            };
            arena_first(
                &mut self.radar,
                &mut self.radar_provinces,
                first.outcome,
                first.provinces_len,
                first.required_provinces,
            );
            ticks[0] = elapsed(begin);
        }

        let volunteer_reads = read_accounting::table(TableId::VolunteerHosts);
        let begin = qpc();
        self.volunteers.outcome = bounded(
            ccountry::walk_volunteer_hosts_with_air_buffer(
                memory,
                base,
                game_state,
                engine_native::walk::Buffer::Growing(&mut self.volunteers.rows),
                true,
            ),
            self.volunteers.rows.len(),
        );
        ticks[1] = elapsed(begin);
        drop(volunteer_reads);
        let sent_reads = read_accounting::table(TableId::SentVolunteers);
        let begin = qpc();
        let rows = &mut self.sent_volunteers.rows;
        self.sent_volunteers.outcome = bounded(
            ccountry::walk_sent_volunteers_buffer(
                memory,
                base,
                game_state,
                engine_native::walk::Buffer::Growing(rows),
            ),
            self.sent_volunteers.rows.len(),
        );
        ticks[6] = elapsed(begin);
        drop(sent_reads);

        if !self.stationed {
            let outcome = Outcome::Unavailable {
                reason: collector_extended_support::outcomes::NOT_RECORDED,
            };
            self.areas.fail(outcome);
            self.units.fail(outcome);
        }
        let stationed_slot = if self.active[2] { 2 } else { 3 };
        let area_reads = self
            .stationed
            .then(|| read_accounting::table(TABLES[stationed_slot]));
        let begin = qpc();
        let first = self.stationed.then(|| {
            ctheatre::walk_stationed_buffers(
                memory,
                base,
                game_state,
                ctheatre::StationedStorage {
                    areas_active: self.active[2],
                    areas: engine_native::walk::Buffer::Growing(&mut self.areas.rows),
                    area_provinces: engine_native::walk::Buffer::Growing(
                        &mut self.area_provinces.first,
                    ),
                    units: engine_native::walk::Buffer::Growing(&mut self.units.rows),
                },
            )
        });
        if let Some(first) = first {
            if self.active[2] {
                arena_first(
                    &mut self.areas,
                    &mut self.area_provinces,
                    first.areas,
                    first.provinces_len,
                    first.required_provinces,
                );
            } else {
                self.areas.outcome = SKIPPED;
            }
            self.units.outcome = bounded(first.units, self.units.rows.len());
            ticks[stationed_slot] = elapsed(begin);
        }
        drop(area_reads);

        if self.intel_raw_mode {
            ticks[4] = 0;
        } else {
            let _reads = read_accounting::table(TableId::UnitIntel);
            let begin = qpc();
            self.intel.outcome = bounded(
                unit_viewer_entry::walk_with_divisions_buffer(
                    memory,
                    base,
                    game_state,
                    engine_native::walk::Buffer::Growing(&mut self.intel.rows),
                    None,
                ),
                self.intel.rows.len(),
            );
            ticks[4] = elapsed(begin);
        }

        if self.registrations_raw_mode {
            ticks[5] = 0;
        } else {
            let _reads = read_accounting::table(TableId::AirBaseRegistrations);
            let begin = qpc();
            let first = cair_base::walk_registrations(
                memory,
                base,
                game_state,
                cair_base::RegistrationBuffers {
                    registrations: &mut self.registrations.rows,
                    wings: &mut self.wings.first,
                    resolved: &mut self.resolved,
                },
            );
            self.required_links = first.required_links;
            arena_first(
                &mut self.registrations,
                &mut self.wings,
                first.outcome,
                first.wings_len,
                first.required_wings,
            );
            ticks[5] = elapsed(begin);
        }
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.radar.outcome,
            self.volunteers.outcome,
            self.areas.outcome,
            self.units.outcome,
            self.intel.outcome,
            self.registrations.outcome,
            self.sent_volunteers.outcome,
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
        mut projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        use collector_projection::projection::scalar_table;
        let outcomes = self.outcomes();
        [
            self.radar_table(outcomes[0], projection.as_deref_mut()),
            scalar_table::<tables::VolunteerHosts, _>(
                projection.as_deref_mut(),
                outcomes[1],
                &self.volunteers.rows,
                |r| (u128::from(r.country) << 32) | u128::from(r.position),
                |r| rows::VolunteerHostRow {
                    country: r.country,
                    position: r.position,
                    host: r.host,
                    deployed_divisions: r.deployed_divisions,
                    air_capacity_modifier: (r.air_capacity_modifier).into(),
                },
            ),
            self.area_table(outcomes[2], projection.as_deref_mut()),
            scalar_table::<tables::StationedUnits, _>(
                projection.as_deref_mut(),
                outcomes[3],
                &self.units.rows,
                |r| (u128::from(r.theatre) << 32) | u128::from(r.ordinal),
                |r| rows::StationedUnitRow {
                    theatre: r.theatre,
                    ordinal: r.ordinal,
                    unit: r.unit,
                },
            ),
            scalar_table::<tables::UnitIntel, _>(
                projection.as_deref_mut(),
                outcomes[4],
                &self.intel.rows,
                |r| (u128::from(r.unit) << 32) | u128::from(r.ordinal),
                |r| rows::UnitIntelRow {
                    unit: r.unit,
                    ordinal: r.ordinal,
                    viewer: r.viewer,
                    level: r.level,
                },
            ),
            self.registration_table(outcomes[5], projection.as_deref_mut()),
            scalar_table::<tables::SentVolunteers, _>(
                projection,
                outcomes[6],
                &self.sent_volunteers.rows,
                |r| (u128::from(r.country) << 32) | u128::from(r.ordinal),
                |r| rows::SentVolunteerRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    division: r.division,
                },
            ),
        ]
    }

    fn radar_table(
        &self,
        outcome: Outcome,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> TableInput {
        if !valid_arena(outcome, &self.radar.rows, &self.radar_provinces) {
            return collector_projection::table_conversion::unavailable(
                "radar coverage vector lies outside the captured arena",
            )
            .into();
        }
        let provinces = |r: &radar_group::Row| {
            &self.radar_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len]
        };
        collector_projection::projection::project_table::<tables::RadarCoverage, _>(
            projection,
            outcome,
            &self.radar.rows,
            ArenaRow::key,
            |r, old| {
                r.country == old.country
                    && r.ordinal == old.ordinal
                    && r.state == old.state
                    && r.symbol.map(|value| value.radius) == old.radius
                    && provinces(r) == old.provinces
            },
            |r| rows::RadarCoverageRow {
                country: r.country,
                ordinal: r.ordinal,
                state: r.state,
                radius: r.symbol.map(|value| value.radius),
                provinces: owned_vec(copy_vec(provinces(r))),
            },
        )
    }

    fn area_table(
        &self,
        outcome: Outcome,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> TableInput {
        if !valid_arena(outcome, &self.areas.rows, &self.area_provinces) {
            return collector_projection::table_conversion::unavailable(
                "controller area province vector lies outside the captured arena",
            )
            .into();
        }
        let provinces = |r: &ccontroller_area::Row| {
            &self.area_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len]
        };
        collector_projection::projection::project_table::<tables::TheatreAreas, _>(
            projection,
            outcome,
            &self.areas.rows,
            ArenaRow::key,
            |r, old| {
                r.theatre == old.theatre
                    && r.ordinal == old.ordinal
                    && r.representative == old.representative
                    && provinces(r) == old.provinces
            },
            |r| rows::TheatreAreaRow {
                theatre: r.theatre,
                ordinal: r.ordinal,
                representative: r.representative,
                provinces: owned_vec(copy_vec(provinces(r))),
            },
        )
    }

    fn registration_table(
        &self,
        outcome: Outcome,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> TableInput {
        if !valid_arena(outcome, &self.registrations.rows, &self.wings) {
            return collector_projection::table_conversion::unavailable(
                "air registration wing vector lies outside the captured arena",
            )
            .into();
        }
        let wings = |r: &cair_base::RegistrationRow| {
            &self.wings.first[r.wings_start..r.wings_start + r.wings_len]
        };
        collector_projection::projection::project_table::<tables::AirBaseRegistrations, _>(
            projection,
            outcome,
            &self.registrations.rows,
            ArenaRow::key,
            |r, old| {
                let inputs = self.carrier.registration(r);
                let fuel = inputs.state_fuel();
                r.base == old.base
                    && r.slot == old.slot
                    && r.capacity_raw == old.capacity_raw
                    && wings(r) == old.wings
                    && inputs.equal(&old.carrier)
                    && old.fuel_demand_raw == rows::Recorded::Value(fuel[0])
                    && old.fuel_allocated_raw == rows::Recorded::Value(fuel[1])
                    && old.transport_planes == rows::Recorded::Value(rows::Retired)
            },
            |r| {
                let inputs = self.carrier.registration(r);
                let fuel = inputs.state_fuel();
                rows::AirBaseRegistrationRow {
                    base: r.base,
                    slot: r.slot,
                    capacity_raw: r.capacity_raw,
                    wings: owned_vec(copy_vec(wings(r))),
                    carrier: inputs.packet(),
                    fuel_demand_raw: rows::Recorded::Value(fuel[0]),
                    fuel_allocated_raw: rows::Recorded::Value(fuel[1]),
                    transport_planes: rows::Recorded::Value(rows::Retired),
                }
            },
        )
    }

    pub fn fail_registrations_capture(&mut self, reason: &'static str) {
        self.registrations.outcome = Outcome::Unavailable { reason };
        self.carrier.reset();
        self.wings.reset();
        self.registrations_raw_mode = false;
    }

    fn prepare_registrations_payload(
        &mut self,
        job: &super::air::AirBasesJob,
    ) -> Result<(), &'static str> {
        if !self.registrations_raw_mode || job.log.overflowed || job.log.fault[1] {
            return Ok(());
        }
        let Some([rows, wings]) = job.registration_demand else {
            return Ok(());
        };
        super::air::carrier::prepare(&mut self.registrations.rows, rows)?;
        super::air::carrier::prepare(&mut self.wings.first, wings)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn decode_registrations_payload(
        &mut self,
        base: usize,
        job: &super::air::AirBasesJob,
        decode_ticks: &mut [u64; super::air::AIR_BASES_N],
    ) {
        self.decode_registrations_with_carrier_payload(
            base,
            job,
            &[],
            &mut super::air::carrier::Bases::new(0),
            decode_ticks,
            None,
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn decode_registrations_with_carrier_payload(
        &mut self,
        base: usize,
        job: &super::air::AirBasesJob,
        base_rows: &[cair_base::Row],
        carrier_bases: &mut super::air::carrier::Bases,
        decode_ticks: &mut [u64; super::air::AIR_BASES_N],
        reuse: Option<&mut (dyn cair_base::RegistrationReuse + '_)>,
    ) {
        let prepared = self.prepare_decode(job, carrier_bases);
        let _ = self.decode_registrations_prepared(
            base,
            job,
            base_rows,
            carrier_bases,
            decode_ticks,
            reuse,
            prepared,
        );
    }

    pub fn prepare_decode(
        &mut self,
        job: &super::air::AirBasesJob,
        bases: &mut super::air::carrier::Bases,
    ) -> Result<(), &'static str> {
        let demand = job.carrier.demand;
        self.prepare_registrations_payload(job)?;
        bases.prepare(demand)?;
        bases.prepare_states(job)?;
        self.carrier.prepare(demand)?;
        self.carrier_scratch.prepare(demand)
    }

    #[allow(clippy::too_many_arguments, reason = "one decode's borrowed parts")]
    pub fn decode_registrations_prepared(
        &mut self,
        base: usize,
        job: &super::air::AirBasesJob,
        base_rows: &[cair_base::Row],
        carrier_bases: &mut super::air::carrier::Bases,
        decode_ticks: &mut [u64; super::air::AIR_BASES_N],
        mut reuse: Option<&mut (dyn cair_base::RegistrationReuse + '_)>,
        prepared: Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        use collector_raw::raw::Recorded;
        if let Err(reason) = prepared {
            carrier_bases.reset();
            self.fail_registrations_capture(reason);
            return Err(reason);
        }
        let log = match Recorded::build(&job.log) {
            Ok(log) => log,
            Err(reason) => {
                carrier_bases.reset();
                self.fail_registrations_capture(reason);
                return Err(reason);
            }
        };
        let recorded = crate::extended::roster_replay::RosterReplay::new(
            &log,
            &job.roster,
            base,
            job.game_state,
        );
        let carrier_log = job.carrier.log.borrow();
        let optional = Recorded::build(&carrier_log);
        let started = qpc();
        carrier_bases.reset();
        self.carrier.reset();
        let optional_countries = job
            .carrier
            .seed
            .and_then(|()| optional.as_ref().map_err(|reason| *reason))
            .and_then(|optional| {
                ccountry::Reader::new(optional, base, job.game_state)
                    .map(|countries| (optional, countries))
            });
        let refusal = optional_countries.as_ref().err().copied();
        let slots = cair_base::SlotMode {
            dense: job.slot_run.dense,
        };
        let original_buffers = cair_base::RegistrationBuffers {
            registrations: &mut self.registrations.rows,
            wings: &mut self.wings.first,
            resolved: &mut self.resolved,
        };
        let first = if let Ok((optional, countries)) = optional_countries {
            let (first, lengths, demand) = {
                let buffers = super::air::carrier::buffers(
                    carrier_bases,
                    &mut self.carrier,
                    &mut self.carrier_scratch,
                );
                let mut carrier = cair_base::CarrierCapture::new(
                    optional,
                    base,
                    job.game_state,
                    &countries,
                    buffers,
                );
                let first = cair_base::walk_registrations_with_carrier_planned(
                    &recorded,
                    base,
                    job.game_state,
                    original_buffers,
                    base_rows,
                    &mut carrier,
                    slots,
                    reuse.as_deref_mut(),
                );
                let visited_hosts = carrier.demand().bases;
                for source in base_rows
                    .iter()
                    .copied()
                    .filter(|source| source.state.is_none())
                    .skip(visited_hosts)
                {
                    carrier.begin_base(0, source.id, true);
                    carrier.authenticated_base(source);
                    carrier.finish_base(false);
                }
                (first, carrier.lengths(), carrier.demand())
            };
            carrier_bases.install(lengths, demand);
            self.carrier.install(lengths, demand);
            carrier_bases.index_rows();
            self.carrier.index_rows();
            first
        } else {
            let demand = cair_base::CarrierDemand {
                failure: refusal,
                ..Default::default()
            };
            carrier_bases.install(cair_base::CarrierLengths::default(), demand);
            self.carrier
                .install(cair_base::CarrierLengths::default(), demand);
            cair_base::walk_registrations_planned(
                &recorded,
                base,
                job.game_state,
                original_buffers,
                slots,
                reuse,
            )
        };
        if let Err(reason) = carrier_bases.decode_states(job, base, base_rows, &mut self.resolved)
        {
            carrier_bases.reset();
            self.fail_registrations_capture(reason);
            return Err(reason);
        }
        let states = self.carrier.install_states(&job.state_fuel);
        self.required_links = first.required_links;
        arena_first(
            &mut self.registrations,
            &mut self.wings,
            first.outcome,
            first.wings_len,
            first.required_wings,
        );
        self.registrations_raw_mode = false;
        if let Err(reason) = states {
            self.fail_registrations_capture(reason);
        }
        decode_ticks[1] += elapsed(started);
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct RegistrationsMemo {
    rows: Vec<cair_base::RegistrationRow>,
    wings: Vec<u64>,
    required_wings: usize,
}

impl RegistrationsMemo {
    pub fn capacity_bytes(&self) -> u64 {
        collector_primitives::raw_memory::vector_bytes(&self.rows)
            + collector_primitives::raw_memory::vector_bytes(&self.wings)
    }
}

impl Captured {
    pub fn registrations_memo_context(&self) -> Option<[usize; 2]> {
        self.registrations_raw_mode
            .then_some([self.registrations.rows.len(), self.wings.first.len()])
    }

    pub fn registrations_memo_complete(&self) -> bool {
        matches!(self.registrations.outcome, Outcome::Rows { .. })
            && valid_arena(
                self.registrations.outcome,
                &self.registrations.rows,
                &self.wings,
            )
    }

    pub fn save_registrations_memo(&self, memo: &mut RegistrationsMemo) {
        debug_assert!(self.registrations_memo_complete());
        memo.rows.clear();
        memo.wings.clear();
        if reserve(&mut memo.rows, self.registrations.outcome.len()).is_err()
            || reserve(&mut memo.wings, self.wings.len).is_err()
        {
            return;
        }
        memo.rows
            .extend_from_slice(&self.registrations.rows[..self.registrations.outcome.len()]);
        memo.wings
            .extend_from_slice(&self.wings.first[..self.wings.len]);
        memo.required_wings = self.wings.required;
    }

    pub fn restore_registrations_memo(&mut self, memo: &RegistrationsMemo) {
        self.registrations.rows[..memo.rows.len()].copy_from_slice(&memo.rows);
        self.registrations.outcome = Outcome::Rows {
            len: memo.rows.len(),
        };
        self.wings.first[..memo.wings.len()].copy_from_slice(&memo.wings);
        self.wings.len = memo.wings.len();
        self.wings.required = memo.required_wings;
        self.registrations_raw_mode = false;
    }
}

#[cfg(test)]
mod registrations_memo_tests;

#[cfg(any(test, feature = "test-support"))]
pub struct RegistrationsSnapshot {
    pub outcome: Outcome,
    pub rows: Vec<cair_base::RegistrationRow>,
    pub wings: Vec<u64>,
}

#[cfg(any(test, feature = "test-support"))]
impl Captured {
    pub fn collect_registrations_direct(
        rows_cap: usize,
        wings_cap: usize,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> RegistrationsSnapshot {
        let capacities = Capacities {
            rows: [4, 4, 4, 4, 4, rows_cap, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: wings_cap,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let first = cair_base::walk_registrations(
            memory,
            base,
            game_state,
            cair_base::RegistrationBuffers {
                registrations: &mut captured.registrations.rows,
                wings: &mut captured.wings.first,
                resolved: &mut captured.resolved,
            },
        );
        arena_first(
            &mut captured.registrations,
            &mut captured.wings,
            first.outcome,
            first.wings_len,
            first.required_wings,
        );
        captured.registrations_snapshot()
    }

    pub fn registrations_snapshot(&self) -> RegistrationsSnapshot {
        RegistrationsSnapshot {
            outcome: self.registrations.outcome,
            rows: self.registrations.rows[..self.registrations.outcome.len()].to_vec(),
            wings: self.wings.first[..self.wings.len].to_vec(),
        }
    }
}

trait ArenaRow<T>: Default + Clone + Copy + PartialEq {
    fn key(&self) -> u128;
    fn span(&self) -> (usize, usize);
}

impl ArenaRow<u32> for radar_group::Row {
    fn key(&self) -> u128 {
        (u128::from(self.country) << 32) | u128::from(self.ordinal)
    }
    fn span(&self) -> (usize, usize) {
        (self.provinces_start, self.provinces_len)
    }
}

impl ArenaRow<u32> for ccontroller_area::Row {
    fn key(&self) -> u128 {
        (u128::from(self.theatre) << 32) | u128::from(self.ordinal)
    }
    fn span(&self) -> (usize, usize) {
        (self.provinces_start, self.provinces_len)
    }
}

impl ArenaRow<u64> for cair_base::RegistrationRow {
    fn key(&self) -> u128 {
        (u128::from(self.base) << 32) | u128::from(self.slot)
    }
    fn span(&self) -> (usize, usize) {
        (self.wings_start, self.wings_len)
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn valid_arena<R: ArenaRow<T>, T>(outcome: Outcome, rows: &[R], arena: &Arena<T>) -> bool {
    arena.len <= arena.first.len()
        && arena.required >= arena.len
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                let (start, len) = row.span();
                start.checked_add(len).is_some_and(|end| end <= arena.len)
            })
        })
}

fn arena_outcome<R: ArenaRow<T>, T>(
    outcome: Outcome,
    rows: &[R],
    buffer: usize,
    len: usize,
    required: usize,
) -> Outcome {
    let outcome = bounded(outcome, rows.len());
    if len <= buffer
        && required >= len
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                let (start, span) = row.span();
                start.checked_add(span).is_some_and(|end| end <= len)
            })
        })
    {
        outcome
    } else {
        Outcome::Unavailable {
            reason: "captured arena range invalid",
        }
    }
}

fn arena_first<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    outcome: Outcome,
    len: usize,
    required: usize,
) {
    captured.outcome = arena_outcome(outcome, &captured.rows, arena.first.len(), len, required);
    arena.len = len;
    arena.required = required;
}

pub const RADAR_N: usize = 1;
pub const INTEL_N: usize = 1;

mod owned;

pub use owned::{
    IntelCopyContext, IntelCopyReport, OwnedFogJob, RadarCopyContext, RadarCopyReport,
    copy_intel_refresh_with_divisions, copy_radar_refresh,
};

pub type RadarJob = OwnedFogJob<radar_group::Row>;
pub type IntelJob = OwnedFogJob<unit_viewer_entry::Row>;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod radar_test_kit;

#[cfg(any(test, feature = "test-support"))]
pub mod intel_test_kit;

#[cfg(test)]
#[path = "fog/scale_tests.rs"]
mod scale_tests;
