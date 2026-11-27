use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const STATE: usize = 0x100;
const COUNTRY: usize = 0x1000;
const TASK: usize = 0x2000;
const STATS: usize = 0x4000;
const META: usize = 0x11000;
const TERRAIN_IMAGE: usize = 0x10_0000;

struct Fixture {
    bytes: Vec<u8>,
    globals: BTreeMap<usize, u8>,
    fault: Option<(usize, usize)>,
    second_fault: Option<(usize, usize)>,
    fail_once: bool,
    failed: Cell<bool>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        if self
            .fault
            .is_some_and(|(first, len)| address < first + len && end > first)
            && (!self.fail_once || !self.failed.replace(true))
        {
            return false;
        }
        if self
            .second_fault
            .is_some_and(|(first, len)| address < first + len && end > first)
        {
            return false;
        }
        if let Some(bytes) = self.bytes.get(address..end) {
            output.copy_from_slice(bytes);
            return true;
        }
        for (index, byte) in output.iter_mut().enumerate() {
            let Some(value) = self.globals.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}
impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        if let Some(output) = self.bytes.get_mut(address..address + bytes.len()) {
            output.copy_from_slice(bytes);
        } else {
            for (index, byte) in bytes.iter().enumerate() {
                self.globals.insert(address + index, *byte);
            }
        }
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn i64(&mut self, address: usize, value: i64) {
        self.put(address, &value.to_le_bytes());
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn accumulator(&mut self, address: usize, data: usize, values: &[(u32, i64)]) {
        self.array(address, data, values.len() as u32);
        for (index, (key, value)) in values.iter().enumerate() {
            self.u32(data + index * 16, *key);
            self.i64(data + index * 16 + 8, *value);
        }
    }
    fn observed(&self, address: usize, len: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&read| read == (address, len))
            .count()
    }
    fn calls(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == address)
            .count()
    }
}

fn fixture() -> Fixture {
    let mut memory = Fixture {
        bytes: vec![0; 0x30000],
        globals: BTreeMap::new(),
        fault: None,
        second_fault: None,
        fail_once: false,
        failed: Cell::new(false),
        reads: RefCell::new(Vec::new()),
    };
    memory.array(STATE + crate::cgame_state::COUNTRIES, 0x800, 2);
    memory.array(STATE + crate::cgame_state::COUNTRY_LINKS, 0x900, 2);
    memory.u64(0x808, COUNTRY as u64);
    memory.u32(0x904, 1);
    memory.u64(COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    memory.u32(COUNTRY + ccountry::ID, 1);
    memory.u32(TASK + cunit::ROSTER_OWNER, 1);
    memory.u32(TASK + cunit::LOGICAL_OWNER, 7);
    memory.u64(TASK + cunit::STAT_BLOCK, STATS as u64);
    memory.u64(STATS, csub_unit_definition::VTABLE_RVA.0 as u64);
    memory.u32(STATS + 8, u32::MAX);
    memory.i64(STATS + 0x1a8, 700_001);
    memory.i64(STATS + mission::STAT_RANGE, 1_000_000);
    memory.put(TASK + cunit::EXILED, &[0xfe, 0xff]);
    memory.u64(metadata::TABLE_RVA.0, META as u64);
    memory
}

fn verified_reader(memory: &Fixture) -> Reader<'_, Fixture> {
    Reader::new(memory, 0, STATE).with_modifier_guard(Ok(()))
}

fn source_reader<M: Memory>(memory: &M, base: usize) -> Reader<'_, M> {
    Reader::new(memory, base, STATE).with_modifier_guard(Ok(()))
}

fn source_terrain_terms<M: Memory>(
    reader: &mut Reader<'_, M>,
    terms: &mut Terms,
    serial: bool,
) -> Result<(), &'static str> {
    if serial {
        reader.serial_terrain_terms(TASK, 0x7000, terms, true, false)
    } else {
        reader.terrain_terms(TASK, 0x7000, terms, true, false)
    }
}

fn source_capture<M: Memory>(
    reader: &mut Reader<'_, M>,
    task: usize,
    hour: i32,
    serial: bool,
) -> (FreshEtaInputs, Result<engine_native_memory::country_identity::Country, &'static str>) {
    if serial {
        let (eta, owner, _) =
            reader.serial_capture_with_mission_risk_union(task, hour, Ok(false), Ok(()), false);
        (eta, owner)
    } else {
        reader.capture(task, hour)
    }
}

fn source_capture_with_risk<M: Memory>(
    reader: &mut Reader<'_, M>,
    hour: i32,
    demand: Result<bool, &'static str>,
    guard: Result<(), &'static str>,
    contributor: bool,
    serial: bool,
) -> SourcePacket {
    if serial {
        reader.serial_capture_with_mission_risk_union(TASK, hour, demand, guard, contributor)
    } else {
        reader.capture_with_mission_risk_union(TASK, hour, demand, guard, contributor)
    }
}

impl<M: Memory> Reader<'_, M> {
    fn serial_location(
        &mut self,
        location: usize,
        risk: bool,
    ) -> Result<(u32, usize), &'static str> {
        let province = gather(self.memory, location, PROVINCE_FIELDS);
        let province = province.memory(self.memory);
        auth(&province, self.base, location, crate::cprovince::VTABLE_RVA)?;
        if risk && self.full_risk {
            self.context.location =
                field::<u32>(&province, location, crate::cprovince::ID).map(Some);
            self.context.controller =
                field::<u32>(&province, location, crate::cprovince::CONTROLLER).map(Some);
        }
        let region =
            field::<u64>(&province, location, crate::cprovince::STRATEGIC_REGION)? as usize;
        let region_fields = gather(self.memory, region, REGION_FIELDS);
        let region_memory = region_fields.memory(self.memory);
        let id = engine_native_parts::cstrategic_region::id(&region_memory, self.base, region)?
            .ok_or("fresh task-force location region absent")?;
        if risk {
            self.context.region = Ok(Some(id));
        }
        let template =
            field::<u64>(&region_memory, region, engine_native_parts::cstrategic_region::TEMPLATE)? as usize;
        let template_fields = gather(self.memory, template, TEMPLATE_FIELDS);
        let template_memory = template_fields.memory(self.memory);
        auth(
            &template_memory,
            self.base,
            template,
            engine_native_parts::cstrategic_region::TEMPLATE_VTABLE_RVA,
        )?;
        let terrain = naval::terrain_from_template_kept(
            &template_memory,
            self.base,
            template,
            &mut self.ocean,
        )?;
        Ok((id, terrain))
    }

    fn serial_fleet_leader(&mut self, fleet: usize) -> Result<Option<usize>, &'static str> {
        let fleet_fields = gather(self.memory, fleet, FLEET_FIELDS);
        let fleet_memory = fleet_fields.memory(self.memory);
        auth(&fleet_memory, self.base, fleet, cfleet::VTABLE_RVA)?;
        let reference = field::<u64>(&fleet_memory, fleet, cfleet::LEADER)?;
        let leader = self.references.resolve(self.memory, reference)?;
        if let Some(leader) = leader {
            let leader_fields = gather(self.memory, leader, LEADER_FIELDS);
            let leader_memory = leader_fields.memory(self.memory);
            auth(
                &leader_memory,
                self.base,
                leader,
                engine_native_parts::cnavy_leader::VTABLE_RVA,
            )?;
            if field::<u64>(&leader_memory, leader, 8)? != reference {
                return Err("fresh task-force fleet leader reference mismatch");
            }
        }
        Ok(leader)
    }

    fn serial_common(
        &mut self,
        task: usize,
        country_values: [Value; VALUE_COUNT],
        eta: bool,
        risk: bool,
    ) -> Result<Terms, &'static str> {
        let mut total = self.effective_terms(country_values, eta, risk);
        let acquired = (|| -> Result<(), &'static str> {
            let location = field::<u64>(self.memory, task, cunit::LOCATION)? as usize;
            if location == 0 && risk {
                self.context.location = Ok(None);
                self.context.region = Ok(None);
                self.context.controller = Ok(None);
            }
            if location != 0 {
                let (id, terrain) = self.serial_location(location, risk)?;
                if risk && self.full_risk {
                    self.context.terrain_speed = self.speed_class.and_then(|class| {
                        if class < 0 {
                            return Err("risk original mixed terrain speed pending");
                        }
                        let (_, source) = naval::terrain_class(self.memory, terrain, class as u32)?;
                        field(self.memory, source, 0x20)
                    });
                }
                let mut terrain_terms = [Ok(0); TERM_COUNT];
                let acquired =
                    self.serial_terrain_terms(task, terrain, &mut terrain_terms, eta, risk);
                sum(&mut total, terrain_terms);
                acquired?;
                let weather = self.weather.get_or_insert_with(|| {
                    cweather_manager::Reader::new(self.memory, self.base, self.state)
                });
                let record = match weather {
                    Ok(weather) => weather.movement_record(id as i32, true)?,
                    Err(reason) => return Err(*reason),
                };
                if record != 0 {
                    let mut weather = self.terms(
                        add(record, crate::cweather_manager::region_weather::MODIFIERS)?,
                        eta,
                        risk,
                    )?;
                    let reduction = self
                        .effective(3, country_values[3])
                        .map(|value| P.wrapping_add(value).clamp(0, P));
                    for value in &mut weather {
                        *value = value.and_then(|value| {
                            reduction.map(|factor| value.wrapping_mul(factor) / P)
                        });
                    }
                    sum(&mut total, weather);
                }
                sum(
                    &mut total,
                    self.terms(add(task, mission::LOCAL_MODIFIERS)?, eta, risk)?,
                );
            }
            let fleet = field::<u64>(self.memory, task, ctask_force::FLEET)? as usize;
            if fleet != 0
                && let Some(leader) = self.serial_fleet_leader(fleet)?
            {
                sum(
                    &mut total,
                    self.certified_terms(
                        add(leader, mission::LEADER_MODIFIERS)?,
                        leader,
                        eta,
                        risk,
                    )?,
                );
            }
            let headquarters = Array::read(
                self.memory,
                add(task, ctask_force::ATTACHED_HEADQUARTERS)?,
                8,
            )
            .ok_or("fresh task-force headquarters invalid")?;
            let mut strongest: Terms = [Ok(0); TERM_COUNT];
            let acquired = engine_native_memory::cpdx_array::pointers(
                self.memory,
                headquarters,
                "fresh task-force headquarters unreadable",
                |_, entry| {
                    let commander =
                        field::<u64>(self.memory, entry, mission::HEADQUARTERS_COMMANDER)? as usize;
                    if commander == 0 {
                        return Ok(());
                    }
                    let reference =
                        field::<u64>(self.memory, commander, mission::COMMANDER_CHARACTER)?;
                    let mut terms = [Ok(0); TERM_COUNT];
                    let acquired = self.headquarters_terms(reference, &mut terms, eta, risk);
                    for (strongest, term) in strongest.iter_mut().zip(terms) {
                        *strongest = strongest.and_then(|value| term.map(|term| value.max(term)));
                    }
                    acquired
                },
            );
            sum(&mut total, strongest);
            acquired?;
            Ok(())
        })();
        if let Err(reason) = acquired {
            sum(&mut total, [Err(reason); TERM_COUNT]);
        }
        Ok(total)
    }

    fn serial_terrain_terms(
        &mut self,
        task: usize,
        terrain: usize,
        total: &mut Terms,
        eta: bool,
        risk: bool,
    ) -> Result<(), &'static str> {
        let ships = Array::read(self.memory, add(task, ctask_force::SHIPS)?, 8)
            .ok_or("fresh task-force ship roster invalid");
        if risk {
            self.ship_header(ships);
            self.context.ships_walked = true;
            if let Err(reason) = ships {
                self.finish_ship_loan(Err(reason));
            }
        }
        let ships = ships?;
        let mut run: Option<(u32, i32)> = None;
        let mixed = risk && self.full_risk && matches!(self.speed_class, Ok(class) if class < 0);
        let mut speed = Ok(0_i64);
        let mut flush = |reader: &mut Self, class, count: i32, total: &mut Terms| {
            let (source, speed_source) = naval::terrain_class(reader.memory, terrain, class)?;
            if mixed {
                speed = speed.and_then(|sum| {
                    field::<i64>(reader.memory, speed_source, 0x20)
                        .map(|value| sum.wrapping_add(i64::from(count).wrapping_mul(value)))
                });
            }
            let terms = reader.terms(add(source, 0x10)?, eta, risk)?;
            sum(
                total,
                terms.map(|value| value.map(|value| i64::from(count).wrapping_mul(value))),
            );
            Ok::<(), &'static str>(())
        };
        let mut sink_result = Ok(());
        let mut visit = |reader: &mut Self, ordinal, ship: &RosterShip<'_>| {
            ship.admit(
                reader.memory,
                reader.ship_vtable,
                task,
                "fresh task-force ship parent mismatch",
            )?;
            let class = ship.get::<u32>(reader.memory, 2);
            if risk {
                let original = reader.ship_loan(ordinal, ship, class);
                sink_result = sink_result.and(original);
            }
            let class = class?;
            match run {
                Some((previous, count)) if previous == class => run = Some((class, count + 1)),
                Some((previous, count)) => {
                    flush(reader, previous, count, total)?;
                    run = Some((class, 1));
                }
                None => run = Some((class, 1)),
            }
            Ok(())
        };
        let memory = self.memory;
        let walked = engine_native_memory::cpdx_array::pointers(
            memory,
            ships,
            "fresh task-force ship pointer unreadable",
            |ordinal, pointer| visit(self, ordinal, &RosterShip::gather(memory, pointer)),
        );
        if risk {
            self.finish_ship_loan(walked.and(sink_result));
        }
        walked?;
        if let Some((class, count)) = run {
            flush(self, class, count, total)?;
        }
        if ships.count == 0 {
            if mixed {
                self.context.terrain_speed = Ok(0);
            }
            return Ok(());
        }
        if mixed {
            self.context.terrain_speed =
                speed.and_then(|value| engine_native_memory::fixed5::fraction(value, i64::from(ships.count) * P));
        }
        *total = (*total).map(|value| {
            value.and_then(|value| engine_native_memory::fixed5::fraction(value, i64::from(ships.count) * P))
        });
        Ok(())
    }

    fn serial_common_inputs(
        &mut self,
        task: usize,
        eta: bool,
        risk: bool,
    ) -> (
        Result<engine_native_memory::country_identity::Country, &'static str>,
        Result<[Value; VALUE_COUNT], &'static str>,
        Terms,
    ) {
        self.context = RiskContext::default();
        let raw_owner = field::<u32>(self.memory, task, cunit::ROSTER_OWNER);
        self.context.roster_owner = raw_owner;
        let roster = raw_owner.and_then(|raw| self.country(raw));
        let country_values = if eta || risk {
            roster.and_then(|country| {
                self.country_values(
                    add(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
                    country.pointer,
                    eta,
                    risk,
                )
            })
        } else {
            self.modifier_guard
                .and(Err("fresh task-force common query unreached"))
        };
        let common = country_values.and_then(|values| self.serial_common(task, values, eta, risk));
        if risk && self.full_risk {
            self.context.targeted_spotting =
                roster.and_then(|roster| self.targeted_spotting(roster, self.context.controller));
        }
        (
            roster,
            country_values,
            common.unwrap_or_else(|reason| [Err(reason); TERM_COUNT]),
        )
    }

    fn serial_capture_with_mission_risk_union(
        &mut self,
        task: usize,
        sample_hour: i32,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
        contributor: bool,
    ) -> (
        FreshEtaInputs,
        Result<engine_native_memory::country_identity::Country, &'static str>,
        MissionRiskInputs,
    ) {
        let stats = self.stats(task);
        let base_speed = stats.and_then(|stats| self.stat(stats, 0x1a8));
        let terrain_class = stats.and_then(|stats| self.stat(stats, 8));
        let range_base = self
            .modifier_guard
            .and_then(|_| stats.and_then(|stats| self.stat::<i64>(stats, mission::STAT_RANGE)));
        let risk_reached = demand == Ok(true) && guard.is_ok();
        self.full_risk = risk_reached;
        self.speed_class = terrain_class;
        let contribution_reached = guard.is_ok() && (risk_reached || contributor);
        let (roster, country_values, terms) =
            self.serial_common_inputs(task, self.modifier_guard.is_ok(), contribution_reached);
        if contribution_reached {
            self.observe_ship_count(task);
        }
        let fuel = if self.modifier_guard.is_ok() || risk_reached {
            self.fuel(task)
        } else {
            self.modifier_guard
                .and(Err("fresh task-force fuel query unreached"))
        };
        let observed_speed_modifier = terms[0].and_then(|value| {
            self.deficit(fuel, Rva(0x033d_f978))
                .map(|deficit| value.wrapping_add(deficit))
        });
        let speed_modifier = self.modifier_guard.and(observed_speed_modifier);
        let modified_range = range_base.and_then(|base| {
            let additive = terms[1]?;
            let factor = terms[2]?.wrapping_add(self.deficit(fuel, Rva(0x033d_fa08))?);
            let underway = self.underway(task, roster?, country_values)?;
            Ok(base
                .wrapping_add(additive)
                .wrapping_mul(P.wrapping_add(factor).wrapping_add(underway).max(P / 10))
                / P)
        });
        (
            FreshEtaInputs {
                sample_hour,
                base_speed,
                speed_modifier,
                terrain_class,
                modified_range,
                exiled: field(self.memory, task, cunit::EXILED),
                strategic_redeployment: field(self.memory, task, cunit::STRATEGIC_REDEPLOYMENT),
            },
            roster,
            self.risk_packet(
                sample_hour,
                demand,
                guard,
                contributor,
                RiskSources {
                    terms,
                    stats,
                    base_speed,
                    speed_modifier: observed_speed_modifier,
                    terrain_class,
                    detached_activity: if risk_reached {
                        field(self.memory, task, ctask_force::DETACHED_ACTIVITY)
                    } else {
                        Ok(0)
                    },
                },
            ),
        )
    }
}

#[test]
fn current_packet_preserves_native_values_and_independent_stats_fields() {
    let mut memory = fixture();
    memory.fault = Some((STATS + 0x1a8, 8));
    let (row, owner) = verified_reader(&memory).capture(TASK, i32::MIN);
    assert_eq!(row.sample_hour, i32::MIN);
    assert!(row.base_speed.is_err());
    assert_eq!(row.terrain_class, Ok(-1));
    assert_eq!(row.speed_modifier, Ok(0));
    assert_eq!(row.modified_range, Ok(1_000_000));
    assert_eq!(
        (row.exiled, row.strategic_redeployment),
        (Ok(0xfe), Ok(0xff))
    );
    assert_eq!((owner.unwrap().id, owner.unwrap().canonical_slot), (1, 1));
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
    assert_eq!(memory.calls(TASK + cunit::ROSTER_OWNER), 1);
}

#[test]
fn a_range_only_fault_keeps_speed_and_class_and_the_guard_decides_its_use() {
    let mut memory = fixture();
    memory.fault = Some((STATS + mission::STAT_RANGE, 8));
    let row = verified_reader(&memory).capture(TASK, 1).0;
    assert_eq!((row.base_speed, row.terrain_class), (Ok(700_001), Ok(-1)));
    assert!(row.modified_range.is_err());
    assert_eq!(row.speed_modifier, Ok(0));
    let memory = {
        let mut memory = mission_risk_fixture();
        memory.fault = Some((STATS + mission::STAT_RANGE, 8));
        memory
    };
    let (eta, _, risk) = Reader::new(&memory, TERRAIN_IMAGE, STATE)
        .with_modifier_guard(Err("ETA layout mismatch"))
        .capture_with_mission_risk(TASK, 25, Ok(true), Ok(()));
    assert_eq!(
        (eta.speed_modifier, eta.modified_range),
        (Err("ETA layout mismatch"), Err("ETA layout mismatch"))
    );
    assert_eq!((eta.base_speed, eta.terrain_class), (Ok(700_001), Ok(-1)));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
}

#[test]
fn a_stat_gather_that_stops_early_reads_the_rest_alone() {
    use engine_native_memory::batch_fallback::{Batch, count};
    let fallbacks = count(Batch::FreshEtaGathers);
    let mut memory = fixture();
    memory.fault = Some((STATS + 0x1a8, 8));
    let row = verified_reader(&memory).capture(TASK, 1).0;
    assert!(row.base_speed.is_err());
    assert_eq!(row.modified_range, Ok(1_000_000));
    assert!(count(Batch::FreshEtaGathers) > fallbacks);
    memory.fail_once = true;
    let row = verified_reader(&memory).capture(TASK, 2).0;
    assert_eq!(row.base_speed, Ok(700_001));
    assert_eq!(row.modified_range, Ok(1_000_000));
}

#[test]
fn stats_root_failure_keeps_common_and_simulation_flags_available() {
    let mut memory = fixture();
    memory.u64(STATS, 1);
    let (row, _) = verified_reader(&memory).capture(TASK, 4);
    assert!(row.base_speed.is_err() && row.terrain_class.is_err() && row.modified_range.is_err());
    assert_eq!(row.speed_modifier, Ok(0));
    assert_eq!(
        (row.exiled, row.strategic_redeployment),
        (Ok(0xfe), Ok(0xff))
    );
}

#[test]
fn metadata_component_and_shared_root_failures_are_retained_without_retry() {
    for fault in [
        META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS,
        metadata::TABLE_RVA.0,
    ] {
        let mut memory = fixture();
        memory.accumulator(
            COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            0x1800,
            &[(KEYS[0], 123), (KEYS[1], 5), (KEYS[2], -100_000)],
        );
        memory.fault = Some((fault, 4));
        memory.fail_once = true;
        let mut reader = verified_reader(&memory);
        let first = reader.capture(TASK, 1).0;
        let second = reader.capture(TASK, 2).0;
        assert!(first.speed_modifier.is_err() && second.speed_modifier.is_err());
        assert_eq!(first.base_speed, Ok(700_001));
        if fault == metadata::TABLE_RVA.0 {
            assert!(first.modified_range.is_err() && second.modified_range.is_err());
        } else {
            assert_eq!(first.modified_range, Ok(100_000));
            assert_eq!(second.modified_range, first.modified_range);
        }
        assert_eq!(memory.calls(fault), 1);
        assert_eq!(memory.calls(metadata::TABLE_RVA.0), 1);
    }
}

fn country_census(log: &[HeldCensus]) -> HeldCensus {
    let address = COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    *log.iter()
        .find(|census| census.receiver == COUNTRY && census.address == address)
        .expect("the certified country census is logged")
}

#[test]
fn a_held_census_whose_certificate_moved_is_read_afresh() {
    let mut memory = fixture();
    let address = COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    memory.accumulator(
        address,
        0x1800,
        &[(KEYS[0], 123), (KEYS[1], 5), (KEYS[2], -100_000)],
    );
    let mut log = Vec::new();
    let _ = verified_reader(&memory)
        .with_censuses(Censuses {
            held: &[],
            certify: &|_: usize| Some((1_u64, 4_u64)),
            log: &mut log,
        })
        .capture(TASK, 1);
    let mut held = log;
    held.sort_by_key(HeldCensus::identity);
    let before = country_census(&held);
    memory.accumulator(
        address,
        0x1800,
        &[(KEYS[0], 250), (KEYS[1], 5), (KEYS[2], -100_000)],
    );
    let current = verified_reader(&memory).capture(TASK, 2).0;
    let reads = memory.calls(address);
    let asked = RefCell::new(Vec::new());
    let certify = |receiver: usize| {
        if receiver == COUNTRY {
            asked.borrow_mut().push(memory.calls(address));
        }
        Some((1_u64, 6_u64))
    };
    let mut next = Vec::new();
    let row = verified_reader(&memory)
        .with_censuses(Censuses {
            held: &held,
            certify: &certify,
            log: &mut next,
        })
        .capture(TASK, 2)
        .0;
    assert_eq!(
        *asked.borrow(),
        [reads],
        "the certificate precedes the read"
    );
    assert_eq!(memory.calls(address), reads + 1, "the moved census is read");
    assert_eq!(
        (row.speed_modifier, row.modified_range),
        (current.speed_modifier, current.modified_range)
    );
    let after = country_census(&next);
    assert_eq!(after.certificate, (1, 6));
    assert_ne!(
        after.values, before.values,
        "the held values are not served"
    );
}

#[test]
fn an_uncertified_census_is_read_and_not_logged() {
    let mut memory = fixture();
    let address = COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    memory.accumulator(address, 0x1800, &[(KEYS[0], 123)]);
    let mut held = Vec::new();
    let _ = verified_reader(&memory)
        .with_censuses(Censuses {
            held: &[],
            certify: &|_: usize| Some((1_u64, 4_u64)),
            log: &mut held,
        })
        .capture(TASK, 1);
    held.sort_by_key(HeldCensus::identity);
    country_census(&held);
    let reads = memory.calls(address);
    let mut log = Vec::new();
    let _ = verified_reader(&memory)
        .with_censuses(Censuses {
            held: &held,
            certify: &|_: usize| -> Option<Certificate> { None },
            log: &mut log,
        })
        .capture(TASK, 2);
    assert_eq!(memory.calls(address), reads + 1);
    assert!(log.iter().all(|census| census.receiver != COUNTRY));
}

#[test]
fn fuel_uses_native_full_default_skips_and_independent_deficit_defines() {
    let mut memory = fixture();
    memory.fault = Some((TASK + mission::FUEL, 8));
    let row = verified_reader(&memory).capture(TASK, 1).0;
    assert_eq!(
        (row.speed_modifier, row.modified_range),
        (Ok(0), Ok(1_000_000))
    );
    assert_eq!(memory.calls(TASK + mission::FUEL), 0);

    memory.fault = Some((0x033d_f978, 8));
    memory.i64(TASK + ctask_force::CURRENT_FUEL_USAGE, 100_000);
    memory.i64(TASK + mission::FUEL, 50_000);
    memory.i64(0x033d_f978, -50_000);
    memory.i64(0x033d_fa08, -200_000);
    let row = verified_reader(&memory).capture(TASK, 2).0;
    assert!(row.speed_modifier.is_err());
    assert_eq!(row.modified_range, Ok(100_000));
    assert_eq!(memory.calls(TASK + mission::FUEL), 1);
}

fn located_fixture() -> Fixture {
    let mut memory = fixture();
    memory.u64(COUNTRY, (TERRAIN_IMAGE + ccountry::VTABLE_RVA.0) as u64);
    memory.u64(
        STATS,
        (TERRAIN_IMAGE + csub_unit_definition::VTABLE_RVA.0) as u64,
    );
    memory.u64(TERRAIN_IMAGE + metadata::TABLE_RVA.0, META as u64);
    let (location, region, template, terrain) = (0x5000, 0x5800, 0x6000, 0x7000);
    memory.u64(TASK + cunit::LOCATION, location as u64);
    memory.u64(
        location,
        (TERRAIN_IMAGE + crate::cprovince::VTABLE_RVA.0) as u64,
    );
    memory.u64(location + crate::cprovince::STRATEGIC_REGION, region as u64);
    memory.u64(
        region,
        (TERRAIN_IMAGE + engine_native_parts::cstrategic_region::VTABLE_RVA.0) as u64,
    );
    memory.u32(region + engine_native_parts::cstrategic_region::ID, 3);
    memory.u64(region + engine_native_parts::cstrategic_region::TEMPLATE, template as u64);
    memory.u64(
        template,
        (TERRAIN_IMAGE + engine_native_parts::cstrategic_region::TEMPLATE_VTABLE_RVA.0) as u64,
    );
    memory.u64(template + 0x130, terrain as u64);
    memory.u64(
        terrain,
        (TERRAIN_IMAGE + engine_native_parts::cterrain_type::VTABLE_RVA.0) as u64,
    );
    memory.put(terrain + 0x10, &[1]);
    memory.u64(terrain + 0x1b8, 0x8500);
    memory.u32(terrain + 0x1b8 + 12, 0);
    memory.put(terrain + 0x1b8 + 16, &[1]);
    memory.put(0x8500, &[1]);
    memory.u32(0x8504, 2);
    memory.u32(0x8508, 0);
    memory.array(terrain + 0x198, 0x9000, 1);
    memory.accumulator(
        0x9038,
        0x9800,
        &[(KEYS[0], 50_000), (KEYS[1], 25_000), (KEYS[2], 10_000)],
    );
    memory.accumulator(
        terrain + 0xb8,
        0x9b00,
        &[(KEYS[0], -20_000), (KEYS[1], -20_000), (KEYS[2], -5_000)],
    );
    memory.array(TASK + ctask_force::SHIPS, 0xa000, 3);
    for (index, class) in [2, 2, 3].into_iter().enumerate() {
        let ship = 0xb000 + index * 0x1000;
        memory.u64(0xa000 + index * 8, ship as u64);
        memory.u64(ship, (TERRAIN_IMAGE + cship::VTABLE_RVA.0) as u64);
        memory.u64(ship + cship::TASK_FORCE, TASK as u64);
        memory.u32(ship + cship::CLASS_DATABASE_ID, class);
    }
    memory.u64(STATE + crate::cgame_state::WEATHER_MANAGER, 0xe000);
    memory.u64(
        0xe000,
        (TERRAIN_IMAGE + cweather_manager::VTABLE_RVA.0) as u64,
    );
    memory
}

#[test]
fn actual_location_terrain_keeps_adjacent_class_runs_and_native_weighting() {
    let memory = located_fixture();
    let terrain = 0x7000;
    let row = Reader::new(&memory, TERRAIN_IMAGE, STATE)
        .with_modifier_guard(Ok(()))
        .capture(TASK, 17)
        .0;
    assert_eq!(row.speed_modifier, Ok(26_666));
    assert_eq!(row.modified_range, Ok(1_060_500));
    assert_eq!(memory.calls(0x9038), 1);
    assert_eq!(memory.calls(0x9028), 0);
    assert_eq!(memory.calls(terrain + 0xb8), 1);
    assert_eq!(memory.calls(terrain + 0xa8), 0);
    assert_eq!(memory.calls(TASK + cunit::LOCATION), 1);
}

#[test]
fn eta_roster_ships_share_a_guarded_record_batch() {
    const FOURTH: usize = 0x20000;
    let capture = |memory: &Fixture| {
        let census = engine_native_memory::read_census::ReadCensus::new(memory);
        let row = Reader::new(&census, TERRAIN_IMAGE, STATE)
            .with_modifier_guard(Ok(()))
            .capture(TASK, 17)
            .0;
        (row.speed_modifier, census.calls())
    };
    let three = located_fixture();
    let mut four = located_fixture();
    four.array(TASK + ctask_force::SHIPS, 0xa000, 4);
    four.u64(0xa018, FOURTH as u64);
    four.u64(FOURTH, (TERRAIN_IMAGE + cship::VTABLE_RVA.0) as u64);
    four.u64(FOURTH + cship::TASK_FORCE, TASK as u64);
    four.u32(FOURTH + cship::CLASS_DATABASE_ID, 3);
    let (speed, with_three) = capture(&three);
    assert_eq!(speed, Ok(26_666));
    let (_, with_four) = capture(&four);
    assert_eq!(with_four, with_three);

    four.fault = Some((FOURTH + cship::TASK_FORCE, 8));
    let row = Reader::new(&four, TERRAIN_IMAGE, STATE)
        .with_modifier_guard(Ok(()))
        .capture(TASK, 17)
        .0;
    assert_eq!(
        row.speed_modifier,
        Err("fresh task-force operand unreadable")
    );
}

fn roster_fixture(count: usize) -> Fixture {
    let mut memory = located_fixture();
    memory.bytes.resize(0x40000 + count * cship::SIZE, 0);
    memory.array(TASK + ctask_force::SHIPS, 0xa000, count as u32);
    for ordinal in 0..count {
        let ship = 0x40000 + ordinal * cship::SIZE;
        memory.u64(0xa000 + ordinal * 8, ship as u64);
        memory.u64(ship, (TERRAIN_IMAGE + cship::VTABLE_RVA.0) as u64);
        memory.u64(ship + cship::TASK_FORCE, TASK as u64);
        memory.u32(ship + cship::CLASS_DATABASE_ID, 3);
        memory.u64(
            ship + cship::HANDLE,
            ((ordinal as u64 + 1) << 32) | u64::from(cship::HANDLE_TYPE),
        );
    }
    memory
}

#[test]
fn eta_roster_record_pages_keep_serial_terms_and_exact_guarded_cost() {
    for count in [0_usize, 1, 3, 4, 63, 64, 65, 129] {
        let memory = roster_fixture(count);
        let capture = |serial| {
            let census = engine_native_memory::read_census::ReadCensus::new(&memory);
            let mut reader = source_reader(&census, TERRAIN_IMAGE);
            let mut terms = [Ok(0); TERM_COUNT];
            let result = source_terrain_terms(&mut reader, &mut terms, serial);
            (
                result,
                terms,
                census.calls(),
                census.spans(),
                census.bytes(),
            )
        };
        let serial = capture(true);
        let batched = capture(false);
        assert_eq!((batched.0, batched.1), (serial.0, serial.1), "{count}");
        assert_eq!((batched.3, batched.4), (serial.3, serial.4), "{count}");
        assert_eq!(serial.2 - batched.2, count - count.div_ceil(64), "{count}");
    }
}

#[test]
fn eta_roster_record_pages_keep_serial_faults_across_batch_boundaries() {
    for ordinal in [0_usize, 63, 64, 128] {
        for (offset, len) in ROSTER_SHIP {
            let capture = |serial, one_shot| {
                let mut memory = roster_fixture(129);
                let ship = 0x40000 + ordinal * cship::SIZE;
                memory.fault = Some((ship + offset, len));
                memory.fail_once = one_shot;
                let mut reader = source_reader(&memory, TERRAIN_IMAGE);
                let mut terms = [Ok(0); TERM_COUNT];
                let result = source_terrain_terms(&mut reader, &mut terms, serial);
                assert!(memory.calls(ship + offset) <= 2);
                (result, terms)
            };
            assert_eq!(
                capture(false, false),
                capture(true, false),
                "{ordinal}+{offset:x}"
            );
            assert!(capture(false, true).0.is_ok(), "{ordinal}+{offset:x}");
        }
    }
    for ordinal in [0_usize, 63, 64, 128] {
        let capture = |serial| {
            let mut memory = roster_fixture(129);
            memory.fault = Some((0xa000 + ordinal * 8, 8));
            let mut reader = source_reader(&memory, TERRAIN_IMAGE);
            let mut terms = [Ok(0); TERM_COUNT];
            (source_terrain_terms(&mut reader, &mut terms, serial), terms)
        };
        assert_eq!(capture(false), capture(true), "pointer {ordinal}");
    }
}

fn counter_source_world(countries: usize) -> (Fixture, Vec<usize>) {
    let mut memory = located_fixture();
    let table = 0x40000;
    let links = table + (countries + 1) * 8;
    let owners = links + (countries + 1) * 4;
    let tasks = owners + countries * 0x600;
    let rosters = tasks + countries * 0x800;
    let ships = rosters + countries * 24;
    memory.bytes.resize(ships + countries * 3 * cship::SIZE, 0);
    memory.u64(TERRAIN_IMAGE + metadata::TABLE_RVA.0, META as u64);
    memory.array(
        STATE + crate::cgame_state::COUNTRIES,
        table,
        countries as u32 + 1,
    );
    memory.array(
        STATE + crate::cgame_state::COUNTRY_LINKS,
        links,
        countries as u32 + 1,
    );
    let mut pointers = Vec::with_capacity(countries);
    for ordinal in 0..countries {
        let owner = owners + ordinal * 0x600;
        let task = tasks + ordinal * 0x800;
        let roster = rosters + ordinal * 24;
        let id = ordinal as u32 + 1;
        memory.u64(table + id as usize * 8, owner as u64);
        memory.u32(links + id as usize * 4, id);
        memory.u64(owner, (TERRAIN_IMAGE + ccountry::VTABLE_RVA.0) as u64);
        memory.u32(owner + ccountry::ID, id);
        memory.u64(task, (TERRAIN_IMAGE + ctask_force::VTABLE_RVA.0) as u64);
        memory.u32(task + cunit::ROSTER_OWNER, id);
        memory.u64(task + cunit::STAT_BLOCK, STATS as u64);
        memory.u64(task + cunit::LOCATION, 0x5000);
        memory.array(task + ctask_force::SHIPS, roster, 3);
        for (member, class) in [2, 2, 3].into_iter().enumerate() {
            let ship = ships + (ordinal * 3 + member) * cship::SIZE;
            memory.u64(roster + member * 8, ship as u64);
            memory.u64(ship, (TERRAIN_IMAGE + cship::VTABLE_RVA.0) as u64);
            memory.u64(ship + cship::TASK_FORCE, task as u64);
            memory.u32(ship + cship::CLASS_DATABASE_ID, class);
            memory.u64(
                ship + cship::HANDLE,
                (((ordinal * 3 + member + 1) as u64) << 32) | u64::from(cship::HANDLE_TYPE),
            );
        }
        pointers.push(task);
    }
    (memory, pointers)
}

#[test]
fn eta_source_reuse_keeps_packets_for_every_country_navy() {
    for countries in [40, 80] {
        let (memory, tasks) = counter_source_world(countries);
        let capture = |serial| {
            memory.reads.borrow_mut().clear();
            let census = engine_native_memory::read_census::ReadCensus::new(&memory);
            let mut reader = source_reader(&census, TERRAIN_IMAGE);
            let rows: Vec<_> = tasks
                .iter()
                .map(|&task| source_capture(&mut reader, task, 17, serial))
                .collect();
            (rows, census.calls())
        };
        let serial = capture(true);
        let reused = capture(false);
        assert_eq!(reused.0, serial.0, "{countries}");
        assert_eq!(reused.0.len(), countries);
        assert!(
            reused.1 + countries * 12 < serial.1,
            "{countries}: {} vs {}",
            reused.1,
            serial.1
        );
        assert_eq!(memory.calls(0x5000), 1);
        assert_eq!(memory.calls(0x7000 + 0x1b8), 2);
        assert!(
            reused
                .0
                .iter()
                .all(|(row, owner)| row.speed_modifier == Ok(26_666) && owner.is_ok())
        );
    }
}

#[test]
fn first_terrain_key_failure_survives_a_later_raw_source_fault_and_stops_graph() {
    let mut memory = located_fixture();
    memory.fault = Some((
        META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS,
        4,
    ));
    memory.second_fault = Some((0x7000 + 0xb8, 16));
    let row = Reader::new(&memory, TERRAIN_IMAGE, STATE)
        .with_modifier_guard(Ok(()))
        .capture(TASK, 4)
        .0;
    assert_eq!(
        row.speed_modifier,
        Err("fresh task-force operand unreadable")
    );
    assert_eq!(
        row.modified_range,
        Err("modifier accumulator header unreadable or invalid")
    );
    assert_eq!(row.base_speed, Ok(700_001));
    assert_eq!(
        memory.calls(META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS),
        1
    );
    assert_eq!(memory.calls(0x7000 + 0xb8), 1);
    assert_eq!(memory.calls(STATE + crate::cgame_state::WEATHER_MANAGER), 0);
    assert_eq!(memory.calls(TASK + ctask_force::ATTACHED_HEADQUARTERS), 0);
}

#[test]
fn unreached_underway_skips_country_and_missing_flag_is_component_local() {
    let mut memory = fixture();
    memory.fault = Some((TASK + cunit::EXILED, 1));
    let row = verified_reader(&memory).capture(TASK, 3).0;
    assert!(row.exiled.is_err());
    assert_eq!(row.strategic_redeployment, Ok(0xff));
    assert_eq!(row.modified_range, Ok(1_000_000));
    assert_eq!(memory.calls(TASK + cunit::EXPEDITIONARY_OWNER), 0);
    assert_eq!(memory.calls(TASK + mission::REPLENISHMENT), 0);
}

#[test]
fn reached_underway_uses_signed_convoy_ratio_and_positive_expeditionary_alias() {
    let mut memory = fixture();
    memory.array(STATE + crate::cgame_state::COUNTRY_LINKS, 0x900, 3);
    memory.u32(0x908, 1);
    memory.accumulator(
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x1800,
        &[(KEYS[4], 50_000)],
    );
    memory.put(TASK + mission::UNDERWAY, &[0xfe]);
    memory.u32(TASK + cunit::EXPEDITIONARY_OWNER, 2);
    memory.u32(TASK + mission::REPLENISHMENT, (-1_i32) as u32);
    memory.u32(TASK + mission::REPLENISHMENT + 4, 2);
    memory.i64(0x033d_fbc0, 100_000);
    let row = verified_reader(&memory).capture(TASK, 9).0;
    assert_eq!(row.modified_range, Ok(250_000));
    assert_eq!(memory.calls(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR), 1);
    assert_eq!(memory.calls(TASK + cunit::EXPEDITIONARY_OWNER), 1);

    memory.u32(TASK + mission::REPLENISHMENT + 4, 0);
    let row = verified_reader(&memory).capture(TASK, 10).0;
    assert_eq!(row.modified_range, Ok(2_500_000));
}

#[test]
fn eligibility_global_failure_is_not_healed_by_a_later_present_key() {
    let mut memory = fixture();
    memory.accumulator(
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x1800,
        &[(KEYS[0], 123), (KEYS[1], 5), (KEYS[2], -100_000)],
    );
    for key in &KEYS[..3] {
        memory.u32(
            META + *key as usize * metadata::STRIDE + metadata::FEATURE_SELECTOR,
            4,
        );
    }
    memory.u64(
        metadata::ELIGIBILITY_CALLBACK_RVA.0,
        metadata::ELIGIBILITY_FLAGS_GETTER_RVA.0 as u64,
    );
    memory.u32(metadata::TRADE_FEATURE_FLAGS_RVA.0, 4);
    memory.fault = Some((metadata::TRADE_FEATURE_FLAGS_RVA.0, 4));
    memory.fail_once = true;
    let mut reader = verified_reader(&memory);
    for hour in [1, 2] {
        let row = reader.capture(TASK, hour).0;
        assert!(row.speed_modifier.is_err() && row.modified_range.is_err());
        assert_eq!(row.base_speed, Ok(700_001));
    }
    assert_eq!(memory.calls(metadata::ELIGIBILITY_CALLBACK_RVA.0), 1);
    assert_eq!(memory.calls(metadata::TRADE_FEATURE_FLAGS_RVA.0), 1);
    let row = verified_reader(&memory).capture(TASK, 3).0;
    assert_eq!(row.speed_modifier, Ok(123));
    assert_eq!(row.modified_range, Ok(100_000));
}

fn headquarters_fixture() -> Fixture {
    use engine_native_parts::creference_registry as registry;
    let mut memory = fixture();
    let (table, entries) = (0x25000, 0x26000);
    let leader_ref = (7_u64 << 32) | u64::from(crate::cunit_leader::REFERENCE_TYPE);
    let characters = [(2_u64 << 32) | 73, (3_u64 << 32) | 73];
    memory.u64(registry::HIGH_ROOT_RVA.0, table as u64);
    memory.u64(registry::LOW_ROOTS_RVA.0 + 73 * 8, table as u64);
    memory.u64(table + registry::ENTRIES, entries as u64);
    memory.u32(table + registry::COUNT, 3);
    memory.u32(table + registry::MASK, 7);
    memory.put(table + registry::OVERFLOW_DISTANCE, &[5]);
    memory.put(
        entries + 13 * 24 + engine_native_parts::reference_bucket::DISTANCE,
        &[255],
    );
    let slot = |reference: u64| {
        let hash = reference
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        (((hash >> 32) as u32 ^ hash as u32) & 7) as usize
    };
    assert_ne!(slot(leader_ref), slot(characters[0]));
    assert_ne!(slot(leader_ref), slot(characters[1]));
    assert_ne!(slot(characters[0]), slot(characters[1]));
    for (reference, pointer) in [
        (leader_ref, 0x8000),
        (characters[0], 0xa000),
        (characters[1], 0xb000),
    ] {
        let bucket = entries + slot(reference) * 24;
        memory.put(bucket + engine_native_parts::reference_bucket::DISTANCE, &[1]);
        memory.u64(bucket + 8, reference);
        memory.u64(bucket + 16, pointer);
    }
    memory.u64(TASK + ctask_force::FLEET, 0x7000);
    memory.u64(0x7000, cfleet::VTABLE_RVA.0 as u64);
    memory.u64(0x7000 + cfleet::LEADER, leader_ref);
    memory.u64(0x8000, engine_native_parts::cnavy_leader::VTABLE_RVA.0 as u64);
    memory.u64(0x8008, leader_ref);
    memory.accumulator(
        0x8000 + mission::LEADER_MODIFIERS,
        0x8800,
        &[(KEYS[0], 100), (KEYS[1], 100)],
    );
    memory.array(TASK + ctask_force::ATTACHED_HEADQUARTERS, 0x9000, 2);
    for (index, &character_ref) in characters.iter().enumerate() {
        let (head, commander, character, leader) = (
            0x9200 + index * 0x100,
            0x9600 + index * 0x100,
            0xa000 + index * 0x1000,
            0xc000 + index * 0x1000,
        );
        memory.u64(0x9000 + index * 8, head as u64);
        memory.u64(head + mission::HEADQUARTERS_COMMANDER, commander as u64);
        memory.u64(commander + mission::COMMANDER_CHARACTER, character_ref);
        memory.u64(character, crate::ccharacter::VTABLE_RVA.0 as u64);
        memory.u64(character + 8, character_ref);
        memory.u64(character + mission::CHARACTER_UNIT_LEADER, leader as u64);
        memory.u64(leader, engine_native_parts::cnavy_leader::VTABLE_RVA.0 as u64);
        memory.u32(leader + mission::UNIT_LEADER_KIND, 2);
        for (ordinal, offset) in mission::UNIT_LEADER_SOURCES.into_iter().enumerate() {
            let source = 0xe000 + index * 0x1000 + ordinal * 0x200;
            memory.u64(leader + offset, source as u64);
            let values = if index == 0 {
                [(KEYS[0], 20), (KEYS[1], -50)]
            } else {
                [(KEYS[0], 30), (KEYS[1], -100)]
            };
            memory.accumulator(
                source + mission::SOURCE_MODIFIERS,
                0x18000 + index * 0x1000 + ordinal * 0x100,
                &values,
            );
        }
    }
    memory
}

#[test]
fn fleet_leader_adds_once_and_headquarters_sum_then_take_maximum_from_zero() {
    let memory = headquarters_fixture();
    let row = verified_reader(&memory).capture(TASK, 12).0;
    assert_eq!(row.speed_modifier, Ok(220));
    assert_eq!(row.modified_range, Ok(1_000_100));
    assert_eq!(memory.calls(0x8000 + mission::LEADER_MODIFIERS), 1);
    assert_eq!(memory.calls(TASK + ctask_force::ATTACHED_HEADQUARTERS), 1);
}

#[test]
fn first_headquarters_key_failure_survives_later_skill_and_entry_raw_faults() {
    for later_fault in [
        0xe200 + mission::SOURCE_MODIFIERS,
        0x9300 + mission::HEADQUARTERS_COMMANDER,
    ] {
        let mut memory = headquarters_fixture();
        memory.accumulator(
            0x8000 + mission::LEADER_MODIFIERS,
            0x8800,
            &[(KEYS[1], 100)],
        );
        memory.fault = Some((
            META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS,
            4,
        ));
        memory.second_fault = Some((later_fault, 8));
        let row = verified_reader(&memory).capture(TASK, 14).0;
        assert_eq!(
            row.speed_modifier,
            Err("fresh task-force operand unreadable")
        );
        assert!(row.modified_range.is_err());
        assert_eq!(row.base_speed, Ok(700_001));
        assert_eq!(
            memory.calls(META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS),
            1
        );
        assert_eq!(memory.calls(later_fault), 1);
    }
}

#[test]
fn motion_capture_reads_fresh_operands_only_for_admitted_current_hour_native_demand() {
    for (hour, route_count, target, path_capacity) in [
        (None, 1, false, 1),
        (Some(17), 0, false, 1),
        (Some(17), 1, true, 1),
        (Some(17), 1, false, 1),
    ] {
        let mut memory = fixture();
        memory.array(STATE + crate::cgame_state::PROVINCES, 0x980, 4);
        memory.array(TASK + cunit::ORDINARY_ROUTE_DATA, 0x4800, route_count);
        memory.u32(0x4800, 2);
        if target {
            memory.u64(0x980 + 2 * 8, 0x5000);
            memory.u64(0x5000, crate::cprovince::VTABLE_RVA.0 as u64);
            memory.u32(0x5000 + crate::cprovince::ID, 2);
            memory.u64(TASK + cunit::MOVEMENT_TARGET, 0x5000);
        }
        let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
        let mut paths = vec![0; path_capacity];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            0,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        );
        if let Some(hour) = hour {
            collector = collector
                .with_fresh_hour(hour)
                .with_fresh_modifier_guard(Ok(()));
        }
        collector.visit(TASK, 7);
        let walk = collector.finish(None);
        let demanded = hour.is_some() && route_count as usize <= path_capacity;
        assert_eq!(rows[0].fresh.is_some(), demanded);
        assert_eq!(rows[0].fresh_owner.is_some(), demanded);
        assert_eq!(
            memory.calls(TASK + cunit::STAT_BLOCK),
            usize::from(demanded)
        );
        assert_eq!(memory.calls(TASK + cunit::LOGICAL_OWNER), 1);
        assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
        assert_eq!(rows[0].logical_owner, 7);
        if demanded {
            assert_eq!(rows[0].fresh.unwrap().sample_hour, 17);
        }
    }
}

#[test]
fn refused_or_unverified_layout_skips_modifier_graph_and_keeps_independent_prefix_and_owner() {
    for guard in [None, Some(Err("fresh task-force modifier layout mismatch"))] {
        let mut memory = fixture();
        memory.array(STATE + crate::cgame_state::PROVINCES, 0x980, 4);
        memory.array(TASK + cunit::ORDINARY_ROUTE_DATA, 0x4800, 1);
        memory.u32(0x4800, 2);
        let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
        let mut paths = vec![0; 1];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            0,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        )
        .with_fresh_hour(18);
        if let Some(guard) = guard {
            collector = collector.with_fresh_modifier_guard(guard);
        }
        collector.visit(TASK, 7);
        let walk = collector.finish(None);
        assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
        assert_eq!(paths, [2]);
        let row = rows[0].fresh.unwrap();
        let reason = guard.map_or("fresh task-force modifier layout unverified", |guard| {
            guard.unwrap_err()
        });
        assert_eq!(
            (row.speed_modifier, row.modified_range),
            (Err(reason), Err(reason))
        );
        assert_eq!((row.base_speed, row.terrain_class), (Ok(700_001), Ok(-1)));
        assert_eq!(
            (row.exiled, row.strategic_redeployment),
            (Ok(0xfe), Ok(0xff))
        );
        assert_eq!(rows[0].fresh_owner.unwrap().unwrap().id, 1);
        assert_eq!(memory.calls(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR), 0);
        assert_eq!(memory.observed(STATS + 0x1a8, 16), 1);
        assert_eq!(memory.observed(STATS + mission::STAT_RANGE, 8), 0);
        assert_eq!(memory.calls(TASK + cunit::LOCATION), 0);
        assert_eq!(memory.calls(TASK + ctask_force::CURRENT_FUEL_USAGE), 0);
        assert_eq!(memory.calls(TASK + mission::UNDERWAY), 0);
    }
}

const RISK_LOCATION: usize = 0x30000;
const RISK_TERRAIN: usize = 0x33000;
const RISK_CLASS_TREE: usize = 0x34138;
const RISK_WEATHER_RECORD: usize = 0x39000 + 3 * crate::cweather_manager::region_weather::STRIDE;
const RISK_COMBINED: i64 = -116_351;
const RISK_METADATA_ROOT: usize = TERRAIN_IMAGE + metadata::TABLE_RVA.0;

fn mission_risk_fixture() -> Fixture {
    use engine_native_parts::creference_registry as registry;
    let mut memory = headquarters_fixture();
    memory.bytes.resize(0x40000, 0);
    memory.u64(COUNTRY, (TERRAIN_IMAGE + ccountry::VTABLE_RVA.0) as u64);
    memory.u64(
        STATS,
        (TERRAIN_IMAGE + csub_unit_definition::VTABLE_RVA.0) as u64,
    );
    memory.u64(RISK_METADATA_ROOT, META as u64);
    memory.u64(TERRAIN_IMAGE + registry::HIGH_ROOT_RVA.0, 0x25000);
    memory.u64(TERRAIN_IMAGE + registry::LOW_ROOTS_RVA.0 + 73 * 8, 0x25000);
    memory.u64(0x7000, (TERRAIN_IMAGE + cfleet::VTABLE_RVA.0) as u64);
    memory.u64(
        0x8000,
        (TERRAIN_IMAGE + engine_native_parts::cnavy_leader::VTABLE_RVA.0) as u64,
    );
    for index in 0..2 {
        let character = 0xa000 + index * 0x1000;
        let leader = 0xc000 + index * 0x1000;
        memory.u64(
            character,
            (TERRAIN_IMAGE + crate::ccharacter::VTABLE_RVA.0) as u64,
        );
        memory.u64(
            leader,
            (TERRAIN_IMAGE + engine_native_parts::cnavy_leader::VTABLE_RVA.0) as u64,
        );
    }
    memory.accumulator(
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x1800,
        &[
            (KEYS[0], 1_000),
            (keys::NAVAL_DETECTION, -120_001),
            (KEYS[1], 100),
            (KEYS[3], -50_000),
        ],
    );
    let (region, template) = (0x31000, 0x32000);
    memory.u64(TASK + cunit::LOCATION, RISK_LOCATION as u64);
    memory.u64(
        RISK_LOCATION,
        (TERRAIN_IMAGE + crate::cprovince::VTABLE_RVA.0) as u64,
    );
    memory.u64(
        RISK_LOCATION + crate::cprovince::STRATEGIC_REGION,
        region as u64,
    );
    memory.u64(
        region,
        (TERRAIN_IMAGE + engine_native_parts::cstrategic_region::VTABLE_RVA.0) as u64,
    );
    memory.u32(region + engine_native_parts::cstrategic_region::ID, 3);
    memory.u64(region + engine_native_parts::cstrategic_region::TEMPLATE, template as u64);
    memory.u64(
        template,
        (TERRAIN_IMAGE + engine_native_parts::cstrategic_region::TEMPLATE_VTABLE_RVA.0) as u64,
    );
    memory.u64(template + 0x130, RISK_TERRAIN as u64);
    memory.u64(
        RISK_TERRAIN,
        (TERRAIN_IMAGE + engine_native_parts::cterrain_type::VTABLE_RVA.0) as u64,
    );
    memory.put(RISK_TERRAIN + 0x10, &[1]);
    memory.u64(RISK_TERRAIN + 0x1b8, 0x34000);
    memory.u32(RISK_TERRAIN + 0x1b8 + 12, 0);
    memory.put(RISK_TERRAIN + 0x1b8 + 16, &[1]);
    memory.put(0x34000, &[1]);
    memory.u32(0x34004, 2);
    memory.u32(0x34008, 0);
    memory.array(RISK_TERRAIN + 0x198, 0x34100, 1);
    memory.accumulator(
        RISK_CLASS_TREE,
        0x34500,
        &[
            (KEYS[0], 300),
            (keys::NAVAL_DETECTION, 6_000),
            (KEYS[1], 900),
        ],
    );
    memory.accumulator(
        RISK_TERRAIN + 0xb8,
        0x34300,
        &[
            (KEYS[0], -300),
            (keys::NAVAL_DETECTION, -3_000),
            (KEYS[1], -900),
        ],
    );
    memory.array(TASK + ctask_force::SHIPS, 0x34400, 3);
    for (index, class) in [2, 2, 3].into_iter().enumerate() {
        let ship = 0x35000 + index * 0x1000;
        memory.u64(0x34400 + index * 8, ship as u64);
        memory.u64(ship, (TERRAIN_IMAGE + cship::VTABLE_RVA.0) as u64);
        memory.u64(ship + cship::TASK_FORCE, TASK as u64);
        memory.u32(ship + cship::CLASS_DATABASE_ID, class);
    }
    let manager = 0x38000;
    memory.u64(STATE + crate::cgame_state::WEATHER_MANAGER, manager as u64);
    memory.u64(
        manager,
        (TERRAIN_IMAGE + cweather_manager::VTABLE_RVA.0) as u64,
    );
    memory.array(manager + cweather_manager::REGIONS, 0x39000, 4);
    memory.u64(
        RISK_WEATHER_RECORD,
        (TERRAIN_IMAGE + crate::cweather_manager::region_weather::VTABLE_RVA.0) as u64,
    );
    memory.u32(RISK_WEATHER_RECORD + crate::cweather_manager::region_weather::ID, 3);
    memory.accumulator(
        RISK_WEATHER_RECORD + crate::cweather_manager::region_weather::MODIFIERS,
        0x3a000,
        &[(KEYS[0], 80), (keys::NAVAL_DETECTION, -80), (KEYS[1], 80)],
    );
    memory.accumulator(
        TASK + mission::LOCAL_MODIFIERS,
        0x3b000,
        &[(KEYS[0], 60), (keys::NAVAL_DETECTION, 70), (KEYS[1], 60)],
    );
    memory.accumulator(
        0x8000 + mission::LEADER_MODIFIERS,
        0x8800,
        &[(KEYS[0], 100), (keys::NAVAL_DETECTION, 500), (KEYS[1], 100)],
    );
    for index in 0..2 {
        for ordinal in 0..mission::UNIT_LEADER_SOURCES.len() {
            let source = 0xe000 + index * 0x1000 + ordinal * 0x200;
            let (speed, detection, range) = if index == 0 {
                (20, -10, -50)
            } else {
                (30, 30, -100)
            };
            memory.accumulator(
                source + mission::SOURCE_MODIFIERS,
                0x18000 + index * 0x1000 + ordinal * 0x100,
                &[
                    (KEYS[0], speed),
                    (keys::NAVAL_DETECTION, detection),
                    (KEYS[1], range),
                ],
            );
        }
    }
    memory
}

fn mission_risk_reader(memory: &Fixture) -> Reader<'_, Fixture> {
    Reader::new(memory, TERRAIN_IMAGE, STATE).with_modifier_guard(Ok(()))
}

type RiskShipObservation = (
    u64,
    u32,
    usize,
    Result<u64, &'static str>,
    Result<u32, &'static str>,
);

#[derive(Debug, Default, PartialEq, Eq)]
struct RiskShipSink {
    headers: Vec<(u64, Result<i32, &'static str>)>,
    ships: Vec<RiskShipObservation>,
    finishes: Vec<(u64, Result<(), &'static str>)>,
    refuse: bool,
}

type SourcePacket = (
    FreshEtaInputs,
    Result<engine_native_memory::country_identity::Country, &'static str>,
    MissionRiskInputs,
);

fn source_risk_fixture() -> Fixture {
    let mut memory = mission_risk_fixture();
    for ordinal in 0..3 {
        memory.u64(
            0x35000 + ordinal * 0x1000 + cship::HANDLE,
            ((ordinal as u64 + 1) << 32) | u64::from(cship::HANDLE_TYPE),
        );
    }
    memory.u64(0x34408, 0x35000);
    memory
}

fn source_packet(memory: &Fixture, serial: bool) -> (SourcePacket, RiskShipSink) {
    let mut sink = RiskShipSink::default();
    let packet = {
        let mut reader = source_reader(memory, TERRAIN_IMAGE).with_risk_ship_observer(&mut sink);
        reader.set_receiver(7);
        source_capture_with_risk(&mut reader, 17, Ok(true), Ok(()), true, serial)
    };
    (packet, sink)
}

#[test]
fn eta_source_batches_match_serial_risk_loans_at_every_persistent_fault() {
    let baseline = source_risk_fixture();
    let healthy = source_packet(&baseline, false);
    assert_eq!(healthy, source_packet(&source_risk_fixture(), true));
    let mut edges: Vec<_> = baseline
        .reads
        .borrow()
        .iter()
        .flat_map(|&(address, len)| [address, address + len.max(1) - 1])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    for edge in edges {
        let mut serial = source_risk_fixture();
        serial.fault = Some((edge, 1));
        let mut batched = source_risk_fixture();
        batched.fault = serial.fault;
        assert_eq!(
            source_packet(&batched, false),
            source_packet(&serial, true),
            "{edge:x}"
        );
    }
}

#[test]
fn eta_source_reuse_keeps_native_refusals_and_the_original_failure_order() {
    for invalid in 0..6 {
        let change = |memory: &mut Fixture| match invalid {
            0 => memory.u64(0x34408, 0),
            1 => memory.u64(0x35000, 1),
            2 => memory.u64(0x35000 + cship::TASK_FORCE, TASK as u64 + 1),
            3 => memory.u64(0x35000 + cship::HANDLE, 0),
            4 => memory.u64(0x8000 + 8, 1),
            5 => memory.u32(RISK_TERRAIN + 0x1b8 + 12, 2),
            _ => unreachable!(),
        };
        let mut serial = source_risk_fixture();
        let mut batched = source_risk_fixture();
        change(&mut serial);
        change(&mut batched);
        assert_eq!(
            source_packet(&batched, false),
            source_packet(&serial, true),
            "{invalid}"
        );
    }
    let mut serial = source_risk_fixture();
    serial.fault = Some((
        META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS,
        4,
    ));
    serial.second_fault = Some((0x37000 + cship::CLASS_DATABASE_ID, 4));
    let mut batched = source_risk_fixture();
    batched.fault = serial.fault;
    batched.second_fault = serial.second_fault;
    assert_eq!(source_packet(&batched, false), source_packet(&serial, true));
}

#[test]
fn eta_successful_source_memos_keep_risk_leaves_fresh_and_end_with_the_capture() {
    let capture = |serial| {
        let memory = source_risk_fixture();
        let census = engine_native_memory::read_census::ReadCensus::new(&memory);
        let mut reader = source_reader(&census, TERRAIN_IMAGE);
        let rows: Vec<_> = (0..3)
            .map(|hour| source_capture_with_risk(&mut reader, hour, Ok(true), Ok(()), true, serial))
            .collect();
        assert_eq!(memory.calls(RISK_LOCATION + crate::cprovince::ID), 3);
        assert_eq!(
            memory.calls(RISK_LOCATION + crate::cprovince::CONTROLLER),
            3
        );
        assert_eq!(memory.calls(RISK_LOCATION), if serial { 3 } else { 1 });
        assert_eq!(memory.calls(0x7000), if serial { 3 } else { 1 });
        assert_eq!(memory.calls(0x8000), if serial { 3 } else { 1 });
        assert_eq!(
            memory.calls(RISK_TERRAIN + 0x1b8),
            if serial { 6 } else { 2 }
        );
        let calls = census.calls();
        let fresh = source_capture_with_risk(
            &mut source_reader(&census, TERRAIN_IMAGE),
            3,
            Ok(true),
            Ok(()),
            true,
            serial,
        );
        assert_eq!(memory.calls(RISK_LOCATION), if serial { 4 } else { 2 });
        (rows, fresh, calls)
    };
    let serial = capture(true);
    let reused = capture(false);
    assert_eq!((reused.0, reused.1), (serial.0, serial.1));
    assert!(reused.2 + 25 < serial.2, "{} vs {}", reused.2, serial.2);
}

#[test]
fn eta_source_memos_keep_no_failed_selector_or_identity() {
    for (fault, watched) in [
        ((RISK_LOCATION, 8), RISK_LOCATION),
        ((RISK_TERRAIN + 0x1b8, 17), RISK_TERRAIN + 0x1b8),
        ((0x7000, 8), 0x7000),
        ((0x8008, 8), 0x8008),
    ] {
        let memory = {
            let mut memory = source_risk_fixture();
            memory.fault = Some(fault);
            memory
        };
        let mut reader = source_reader(&memory, TERRAIN_IMAGE);
        let first = reader.capture(TASK, 17);
        let second = reader.capture(TASK, 17);
        assert_eq!(first, second);
        assert_eq!(memory.calls(watched), 2, "{watched:x}");
    }
}
impl OriginalRiskShips for RiskShipSink {
    fn begin(&mut self, receiver: u64, count: Result<i32, &'static str>) {
        self.headers.push((receiver, count));
    }
    fn ship(
        &mut self,
        receiver: u64,
        ordinal: u32,
        pointer: usize,
        handle: Result<u64, &'static str>,
        class: Result<u32, &'static str>,
    ) -> Result<(), &'static str> {
        self.ships.push((receiver, ordinal, pointer, handle, class));
        if self.refuse {
            Err("original ship numerical arena exhausted")
        } else {
            Ok(())
        }
    }
    fn finish(&mut self, receiver: u64, original: Result<(), &'static str>) {
        self.finishes.push((receiver, original));
    }
}

#[test]
fn original_risk_ship_sink_preserves_repeated_handles_and_one_original_header() {
    let mut memory = mission_risk_fixture();
    let handle = (4_u64 << 32) | u64::from(cship::HANDLE_TYPE);
    memory.u64(0x35008, handle);
    memory.u64(0x37008, handle.wrapping_add(1_u64 << 32));
    memory.u64(0x34408, 0x35000);
    let mut sink = RiskShipSink::default();
    let packet = {
        let mut reader = mission_risk_reader(&memory).with_risk_ship_observer(&mut sink);
        reader.set_receiver(7);
        reader.capture_mission_risk(TASK, 17, Ok(true), Ok(()))
    };
    assert_eq!(packet.ship_count, Ok(Some(3)));
    assert_eq!(sink.headers, vec![(7, Ok(3))]);
    assert_eq!(sink.finishes, vec![(7, Ok(()))]);
    assert_eq!(
        sink.ships
            .iter()
            .map(|row| (row.1, row.2, row.3, row.4))
            .collect::<Vec<_>>(),
        vec![
            (0, 0x35000, Ok(handle), Ok(2)),
            (1, 0x35000, Ok(handle), Ok(2)),
            (2, 0x37000, Ok(handle.wrapping_add(1_u64 << 32)), Ok(3))
        ]
    );
    assert_eq!(memory.calls(TASK + ctask_force::SHIPS), 1);
    assert_eq!(memory.calls(0x35000 + cship::CLASS_DATABASE_ID), 2);
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
}

#[test]
fn risk_ship_sink_capacity_failure_keeps_common_graph_and_original_demand() {
    let memory = mission_risk_fixture();
    let mut sink = RiskShipSink {
        refuse: true,
        ..Default::default()
    };
    let packet = {
        let mut reader = mission_risk_reader(&memory).with_risk_ship_observer(&mut sink);
        reader.set_receiver(7);
        reader.capture_mission_risk(TASK, 17, Ok(true), Ok(()))
    };
    assert!(packet.combined_detection.is_ok());
    assert_eq!(packet.ship_count, Ok(Some(3)));
    assert_eq!(sink.ships.len(), 3);
    assert_eq!(
        sink.finishes,
        vec![(7, Err("original ship numerical arena exhausted"))]
    );
    assert_eq!(memory.calls(TASK + ctask_force::SHIPS), 1);
}

#[test]
fn failed_original_ship_header_is_not_retried_for_the_carrier_sink() {
    let mut memory = mission_risk_fixture();
    memory.fault = Some((TASK + ctask_force::SHIPS, 16));
    memory.fail_once = true;
    let mut sink = RiskShipSink::default();
    let packet = {
        let mut reader = mission_risk_reader(&memory).with_risk_ship_observer(&mut sink);
        reader.set_receiver(7);
        reader.capture_mission_risk(TASK, 17, Ok(true), Ok(()))
    };
    assert!(packet.ship_count.is_err());
    assert_eq!(sink.headers.len(), 1);
    assert!(sink.headers[0].1.is_err());
    assert_eq!(sink.finishes.len(), 1);
    assert!(sink.finishes[0].1.is_err());
    assert!(sink.ships.is_empty());
    assert_eq!(memory.calls(TASK + ctask_force::SHIPS), 1);
}

#[test]
fn mixed_actual_terrain_speed_borrows_original_class_runs_independently() {
    let mut memory = mission_risk_fixture();
    memory.i64(0x34100 + 0xe8 + 0x20, 7_000);
    memory.i64(RISK_TERRAIN + 0x168 + 0x20, 13_000);
    let packet = mission_risk_reader(&memory).capture_mission_risk(TASK, 17, Ok(true), Ok(()));
    assert_eq!(packet.actual_terrain_speed, Ok(Some(9_000)));
    assert_eq!(memory.calls(0x34100 + 0xe8 + 0x20), 1);
    assert_eq!(memory.calls(RISK_TERRAIN + 0x168 + 0x20), 1);
    assert_eq!(memory.calls(TASK + ctask_force::SHIPS), 1);
    assert_eq!(memory.calls(STATS + 8), 1);

    memory.fault = Some((0x34100 + 0xe8 + 0x20, 8));
    let packet = mission_risk_reader(&memory).capture_mission_risk(TASK, 18, Ok(true), Ok(()));
    assert!(packet.actual_terrain_speed.is_err());
    assert!(packet.combined_detection.is_ok());
    assert!(packet.speed_modifier.is_ok());
}

#[test]
fn targeted_spotting_uses_original_roster_and_native_zero_or_unlinked_slot() {
    for controller in [0, 2] {
        let mut memory = mission_risk_fixture();
        memory.bytes.resize(0x47000, 0);
        memory.array(STATE + crate::cgame_state::COUNTRY_LINKS, 0x900, 3);
        memory.u32(RISK_LOCATION + crate::cprovince::CONTROLLER, controller);
        let (diplomacy, status) = (0x43000, 0x44000);
        memory.u64(COUNTRY + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        memory.u64(
            diplomacy,
            (TERRAIN_IMAGE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64,
        );
        memory.u64(
            diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            COUNTRY as u64,
        );
        memory.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS, 0x45000, 2);
        memory.u64(0x45000, status as u64);
        memory.u64(
            status,
            (TERRAIN_IMAGE + crate::crelation::status::VTABLE_RVA.0) as u64,
        );
        memory.u64(
            status + crate::crelation::status::DIPLOMACY_STATUS,
            diplomacy as u64,
        );
        memory.u32(status + crate::crelation::status::TARGET_COUNTRY, 0);
        memory.accumulator(
            status + crate::crelation::status::MODIFIER + 0x10,
            0x46000,
            &[(TARGETED_SPOTTING_KEY, -17_001)],
        );
        let packet = mission_risk_reader(&memory).capture_mission_risk(TASK, 17, Ok(true), Ok(()));
        assert_eq!(packet.targeted_spotting_modifier, Ok(Some(-17_001)));
        assert_eq!(memory.calls(TASK + cunit::ROSTER_OWNER), 1);
        assert_eq!(
            memory.calls(RISK_LOCATION + crate::cprovince::CONTROLLER),
            1
        );
        assert_eq!(
            memory.calls(status + crate::crelation::status::MODIFIER + 0x10),
            1
        );
        if controller == 0 {
            assert_eq!(memory.calls(0x900), 0);
        }
    }
}

fn motion_mission_risk_fixture(route_count: u32) -> Fixture {
    let mut memory = mission_risk_fixture();
    memory.array(STATE + crate::cgame_state::PROVINCES, 0x980, 4);
    memory.array(TASK + cunit::ORDINARY_ROUTE_DATA, 0x4800, route_count);
    memory.u32(0x4800, 2);
    memory
}

fn assert_mission_risk_graph_read_once(memory: &Fixture) {
    for address in [
        TASK + cunit::ROSTER_OWNER,
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        TASK + cunit::LOCATION,
        TASK + ctask_force::SHIPS,
        RISK_CLASS_TREE,
        RISK_TERRAIN + 0xb8,
        STATE + crate::cgame_state::WEATHER_MANAGER,
        RISK_WEATHER_RECORD + crate::cweather_manager::region_weather::MODIFIERS,
        TASK + mission::LOCAL_MODIFIERS,
        0x8000 + mission::LEADER_MODIFIERS,
        TASK + ctask_force::ATTACHED_HEADQUARTERS,
        RISK_METADATA_ROOT,
    ] {
        assert_eq!(
            memory.calls(address),
            1,
            "duplicate graph read at {address:x}"
        );
    }
    for wrapper in [RISK_CLASS_TREE - 0x10, RISK_TERRAIN + 0xa8] {
        assert_eq!(
            memory.calls(wrapper),
            0,
            "wrapper read as accumulator at {wrapper:x}"
        );
    }
    for index in 0..2 {
        for ordinal in 0..mission::UNIT_LEADER_SOURCES.len() {
            let source = 0xe000 + index * 0x1000 + ordinal * 0x200;
            assert_eq!(memory.calls(source + mission::SOURCE_MODIFIERS), 1);
        }
    }
}

fn assert_mission_risk_skips_eta_fields(memory: &Fixture) {
    assert_eq!(memory.observed(STATS + mission::STAT_RANGE, 8), 0);
    for address in [
        TASK + mission::UNDERWAY,
        TASK + cunit::EXILED,
        TASK + cunit::STRATEGIC_REDEPLOYMENT,
    ] {
        assert_eq!(
            memory.calls(address),
            0,
            "risk-only reached ETA field at {address:x}"
        );
    }
}

#[test]
fn mission_risk_combines_country_terrain_weather_local_leader_and_headquarters_once() {
    let memory = mission_risk_fixture();
    let (eta, owner, risk) =
        mission_risk_reader(&memory).capture_with_mission_risk(TASK, 23, Ok(true), Ok(()));
    assert_eq!((eta.sample_hour, risk.sample_hour), (23, 23));
    assert_eq!(owner.unwrap().id, 1);
    assert_eq!(eta.speed_modifier, Ok(1_420));
    assert_eq!(eta.modified_range, Ok(1_000_600));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert!(risk.combined_detection.unwrap().unwrap() < -100_000);
    assert_mission_risk_graph_read_once(&memory);
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
    assert_eq!(
        memory.calls(META + keys::NAVAL_DETECTION as usize * metadata::STRIDE + metadata::FLAGS),
        1
    );
}

#[test]
fn eta_source_reuse_keeps_changed_risk_demand_and_explicit_speed_class() {
    let capture = |serial| {
        let mut memory = source_risk_fixture();
        memory.u32(STATS + 8, 2);
        memory.i64(0x34100 + 0xe8 + 0x20, 7_000);
        let mut reader = source_reader(&memory, TERRAIN_IMAGE);
        [
            source_capture_with_risk(&mut reader, 17, Ok(false), Ok(()), false, serial),
            source_capture_with_risk(&mut reader, 17, Ok(true), Ok(()), false, serial),
            source_capture_with_risk(&mut reader, 17, Ok(false), Ok(()), true, serial),
            source_capture_with_risk(&mut reader, 17, Ok(true), Err("risk guard"), false, serial),
        ]
    };
    let serial = capture(true);
    let reused = capture(false);
    assert_eq!(reused, serial);
    assert_eq!(reused[1].2.actual_terrain_speed, Ok(Some(7_000)));
    assert_eq!(reused[2].2.coordination, Ok(None));
    assert_eq!(reused[3].2.combined_detection, Err("risk guard"));
}

#[test]
fn mission_risk_metadata_failures_remain_cached_without_poisoning_independent_eta_keys() {
    let risk_flags = META + keys::NAVAL_DETECTION as usize * metadata::STRIDE + metadata::FLAGS;
    for fault in [risk_flags, metadata::TABLE_RVA.0] {
        let mut memory = fixture();
        memory.accumulator(
            COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            0x1800,
            &[(KEYS[0], 5), (keys::NAVAL_DETECTION, 123), (KEYS[1], 9)],
        );
        memory.fault = Some((fault, 4));
        memory.fail_once = true;
        let mut reader = verified_reader(&memory);
        for hour in [1, 2] {
            let (eta, _, risk) = reader.capture_with_mission_risk(TASK, hour, Ok(true), Ok(()));
            assert_eq!(risk.sample_hour, hour);
            assert!(risk.combined_detection.is_err());
            assert_eq!(eta.base_speed, Ok(700_001));
            if fault == risk_flags {
                assert_eq!(eta.speed_modifier, Ok(5));
                assert_eq!(eta.modified_range, Ok(1_000_009));
            } else {
                assert!(eta.speed_modifier.is_err() && eta.modified_range.is_err());
            }
        }
        assert_eq!(memory.calls(fault), 1);
        assert_eq!(memory.calls(metadata::TABLE_RVA.0), 1);
        let (_, _, risk) =
            verified_reader(&memory).capture_with_mission_risk(TASK, 3, Ok(true), Ok(()));
        assert_eq!(risk.combined_detection, Ok(Some(123)));
        assert_eq!(memory.calls(fault), 2);
    }
}

#[test]
fn mission_risk_false_demand_failed_demand_and_guard_skip_risk_metadata() {
    for (demand, guard, expected) in [
        (Ok(false), Ok(()), Ok(None)),
        (
            Err("original risk admission unreadable"),
            Ok(()),
            Err("original risk admission unreadable"),
        ),
        (
            Ok(true),
            Err("risk layout mismatch"),
            Err("risk layout mismatch"),
        ),
    ] {
        let memory = mission_risk_fixture();
        let (eta, _, risk) =
            mission_risk_reader(&memory).capture_with_mission_risk(TASK, 24, demand, guard);
        assert_eq!(risk.combined_detection, expected);
        assert_eq!(
            (eta.speed_modifier, eta.modified_range),
            (Ok(1_420), Ok(1_000_600))
        );
        assert_eq!(
            memory.calls(
                META + keys::NAVAL_DETECTION as usize * metadata::STRIDE
                    + metadata::FEATURE_SELECTOR
            ),
            0
        );
        assert_eq!(
            memory
                .calls(META + keys::NAVAL_DETECTION as usize * metadata::STRIDE + metadata::FLAGS),
            0
        );
    }
}

#[test]
fn mission_risk_guard_succeeds_independently_of_eta_layout_refusal() {
    let memory = mission_risk_fixture();
    let (eta, owner, risk) = Reader::new(&memory, TERRAIN_IMAGE, STATE)
        .with_modifier_guard(Err("ETA layout mismatch"))
        .capture_with_mission_risk(TASK, 25, Ok(true), Ok(()));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(owner.unwrap().id, 1);
    assert_eq!(
        (eta.speed_modifier, eta.modified_range),
        (Err("ETA layout mismatch"), Err("ETA layout mismatch"))
    );
    assert_eq!((eta.base_speed, eta.terrain_class), (Ok(700_001), Ok(-1)));
    assert_eq!(memory.observed(STATS + 0x1a8, 16), 1);
    assert_eq!(memory.observed(STATS + mission::STAT_RANGE, 8), 0);
    assert_eq!(memory.calls(TASK + ctask_force::CURRENT_FUEL_USAGE), 1);
    assert_eq!(
        memory.calls(META + KEYS[0] as usize * metadata::STRIDE + metadata::FLAGS),
        1
    );
    assert_mission_risk_graph_read_once(&memory);
}

#[test]
fn mission_risk_shared_pointer_failure_stops_later_sources_without_retry() {
    let mut memory = mission_risk_fixture();
    memory.fault = Some((TASK + cunit::LOCATION, 8));
    let (eta, _, risk) =
        mission_risk_reader(&memory).capture_with_mission_risk(TASK, 26, Ok(true), Ok(()));
    let reason = "fresh task-force operand unreadable";
    assert_eq!(risk.combined_detection, Err(reason));
    assert_eq!(
        (eta.speed_modifier, eta.modified_range),
        (Err(reason), Err(reason))
    );
    assert_eq!(memory.calls(TASK + cunit::LOCATION), 1);
    assert_eq!(memory.calls(TASK + ctask_force::SHIPS), 1);
    for address in [
        STATE + crate::cgame_state::WEATHER_MANAGER,
        TASK + mission::LOCAL_MODIFIERS,
        TASK + ctask_force::FLEET,
        TASK + ctask_force::ATTACHED_HEADQUARTERS,
    ] {
        assert_eq!(memory.calls(address), 0);
    }
}

#[test]
fn mission_risk_survives_independent_eta_stats_root_failure() {
    let mut memory = mission_risk_fixture();
    memory.fault = Some((TASK + cunit::STAT_BLOCK, 8));
    let (eta, _, risk) =
        mission_risk_reader(&memory).capture_with_mission_risk(TASK, 27, Ok(true), Ok(()));
    assert!(eta.base_speed.is_err() && eta.terrain_class.is_err() && eta.modified_range.is_err());
    assert_eq!(eta.speed_modifier, Ok(1_420));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_mission_risk_graph_read_once(&memory);
}

#[test]
fn mission_risk_only_reads_one_graph_and_keeps_speed_stats_failures_independent() {
    let mut memory = mission_risk_fixture();
    memory.fault = Some((TASK + cunit::STAT_BLOCK, 8));
    memory.second_fault = Some((TASK + mission::FUEL, 8));
    let risk =
        Reader::new(&memory, TERRAIN_IMAGE, STATE).capture_mission_risk(TASK, 28, Ok(true), Ok(()));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_mission_risk_graph_read_once(&memory);
    assert_mission_risk_skips_eta_fields(&memory);
    assert!(risk.speed_base.is_err());
    assert_eq!(risk.speed_modifier, Ok(Some(1_420)));
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
    assert_eq!(memory.calls(TASK + ctask_force::CURRENT_FUEL_USAGE), 1);
    for key in &KEYS[1..3] {
        assert_eq!(
            memory.calls(META + *key as usize * metadata::STRIDE + metadata::FLAGS),
            0
        );
    }
}

#[test]
fn motion_mission_risk_idle_admission_shares_eta_graph_without_an_ordinary_route() {
    let memory = motion_mission_risk_fixture(0);
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(29)
    .with_fresh_modifier_guard(Ok(()));
    let risk = collector
        .visit_with_mission_risk(TASK, 7, Ok(true), Ok(()))
        .unwrap();
    let walk = collector.finish(None);
    assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
    assert_eq!((walk.paths_len, walk.required_paths), (0, 0));
    assert_eq!(rows[0].ordinary_len, 0);
    assert_eq!(rows[0].fresh.unwrap().sample_hour, 29);
    assert_eq!(rows[0].fresh.unwrap().speed_modifier, Ok(1_420));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(risk.sample_hour, 29);
    assert_mission_risk_graph_read_once(&memory);
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
}

#[test]
fn motion_mission_risk_missing_hour_refuses_packet_without_reading_fresh_graph() {
    let memory = motion_mission_risk_fixture(0);
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    );
    assert_eq!(
        collector.visit_with_mission_risk(TASK, 7, Ok(true), Ok(())),
        Err("fresh task-force risk sample hour unavailable")
    );
    let walk = collector.finish(None);
    assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
    assert!(rows[0].fresh.is_none());
    assert_eq!(memory.calls(TASK + cunit::ROSTER_OWNER), 0);
    assert_eq!(memory.calls(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR), 0);
    assert_eq!(memory.calls(RISK_METADATA_ROOT), 0);
    assert_mission_risk_skips_eta_fields(&memory);
}

#[test]
fn motion_mission_risk_failed_original_admission_keeps_exact_refusal_and_eta_packet() {
    let memory = motion_mission_risk_fixture(0);
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(30)
    .with_fresh_modifier_guard(Ok(()));
    let risk = collector
        .visit_with_mission_risk(TASK, 7, Err("mission backlink unreadable"), Ok(()))
        .unwrap();
    let walk = collector.finish(None);
    assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
    assert_eq!(risk.combined_detection, Err("mission backlink unreadable"));
    assert_eq!(rows[0].fresh.unwrap().speed_modifier, Ok(1_420));
    assert_eq!(
        memory.calls(META + keys::NAVAL_DETECTION as usize * metadata::STRIDE + metadata::FLAGS),
        0
    );
}

#[test]
fn motion_mission_risk_route_failure_keeps_one_risk_only_graph_and_motion_refusal() {
    let mut memory = motion_mission_risk_fixture(1);
    memory.fault = Some((TASK + cunit::ORDINARY_ROUTE_DATA, 16));
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![0; 1];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(31)
    .with_fresh_modifier_guard(Ok(()));
    let risk = collector
        .visit_with_mission_risk(TASK, 7, Ok(true), Ok(()))
        .unwrap();
    let walk = collector.finish(None);
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(
        walk.outcome,
        engine_native_memory::walk::Outcome::Unavailable {
            reason: "unit route invalid or longer than the province roster"
        }
    );
    assert!(rows[0].fresh.is_none());
    assert_mission_risk_graph_read_once(&memory);
    assert_mission_risk_skips_eta_fields(&memory);
}

#[test]
fn motion_mission_risk_prior_motion_failure_does_not_retry_prefix_or_lose_risk() {
    const BAD_TASK: usize = 0x28000;
    let mut memory = motion_mission_risk_fixture(0);
    memory.fault = Some((BAD_TASK + cunit::ORDINARY_ROUTE_DATA, 16));
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(33)
    .with_fresh_modifier_guard(Ok(()));
    collector.visit(BAD_TASK, 8);
    let risk = collector
        .visit_with_mission_risk(TASK, 7, Ok(true), Ok(()))
        .unwrap();
    let walk = collector.finish(None);
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(
        walk.outcome,
        engine_native_memory::walk::Outcome::Unavailable {
            reason: "unit route invalid or longer than the province roster"
        }
    );
    assert_eq!(memory.calls(BAD_TASK + cunit::ORDINARY_ROUTE_DATA), 1);
    assert_eq!(memory.calls(TASK + cunit::LOGICAL_OWNER), 0);
    assert!(rows[0].fresh.is_none());
    assert_mission_risk_graph_read_once(&memory);
    assert_mission_risk_skips_eta_fields(&memory);
}

#[test]
fn mission_risk_present_native_zero_is_reached_and_demands_metadata() {
    let risk_flags = META + keys::NAVAL_DETECTION as usize * metadata::STRIDE + metadata::FLAGS;
    for (demand, metadata_fault, expected, metadata_reads) in [
        (Ok(true), false, Ok(Some(0)), 1),
        (Ok(false), false, Ok(None), 0),
        (
            Ok(true),
            true,
            Err("fresh task-force operand unreadable"),
            1,
        ),
    ] {
        let mut memory = fixture();
        memory.accumulator(
            COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            0x1800,
            &[(keys::NAVAL_DETECTION, 0)],
        );
        if metadata_fault {
            memory.fault = Some((risk_flags, 4));
        }
        let risk = verified_reader(&memory).capture_mission_risk(TASK, 34, demand, Ok(()));
        assert_eq!(risk.sample_hour, 34);
        assert_eq!(risk.combined_detection, expected);
        assert_eq!(memory.calls(risk_flags), metadata_reads);
        if demand == Ok(false) {
            assert!(memory.reads.borrow().is_empty());
        }
    }
}

#[test]
fn mission_risk_full_modifier_union_and_stat_leaves_share_original_observations() {
    let mut memory = mission_risk_fixture();
    let mut values = vec![
        (0x19, 119),
        (KEYS[0], 300),
        (keys::NAVAL_DETECTION, -120_001),
        (0x55, -55),
        (KEYS[1], 900),
        (0x108, 108),
        (0x19f, -199),
        (KEYS[3], -50_000),
    ];
    values.sort_unstable_by_key(|&(key, _)| key);
    memory.accumulator(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0x1800, &values);
    memory.i64(STATS + 0x138, -138);
    memory.i64(STATS + 0x148, i64::MIN);
    memory.i64(STATS + 0x150, i64::MAX);
    memory.i64(STATS + 0x160, 160);
    let (eta, _, risk) =
        mission_risk_reader(&memory).capture_with_mission_risk(TASK, 39, Ok(true), Ok(()));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(risk.coordination, Ok(Some(119)));
    assert_eq!(risk.spotting_chance, Ok(Some(-55)));
    assert_eq!(risk.night_spotting_chance, Ok(Some(108)));
    assert_eq!(risk.visibility, Ok(Some(-199)));
    assert_eq!(risk.speed_base, eta.base_speed.map(Some));
    assert_eq!(risk.speed_modifier, eta.speed_modifier.map(Some));
    assert_eq!(risk.ship_count, Ok(Some(3)));
    let stats = risk.stats.unwrap().unwrap();
    assert_eq!(
        (
            stats.detection_138,
            stats.visibility_148,
            stats.visibility_150,
            stats.carrier_detection_160
        ),
        (Ok(-138), Ok(i64::MIN), Ok(i64::MAX), Ok(160))
    );
    assert_mission_risk_graph_read_once(&memory);
    for offset in [cunit::STAT_BLOCK, ctask_force::CURRENT_FUEL_USAGE] {
        assert_eq!(memory.calls(TASK + offset), 1);
    }
    for offset in [0x138, 0x148, 0x150, 0x160, 0x1a8] {
        assert_eq!(memory.calls(STATS + offset), 1);
    }
}

#[test]
fn mission_risk_new_key_failure_preserves_detection_eta_and_other_risk_keys() {
    let mut memory = fixture();
    memory.accumulator(
        COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x1800,
        &[
            (0x19, 19),
            (KEYS[0], 29),
            (keys::NAVAL_DETECTION, 42),
            (0x55, 55),
        ],
    );
    let fault = META + 0x55 * metadata::STRIDE + metadata::FLAGS;
    memory.fault = Some((fault, 4));
    memory.fail_once = true;
    let mut reader = verified_reader(&memory);
    for hour in [1, 2] {
        let (eta, _, risk) = reader.capture_with_mission_risk(TASK, hour, Ok(true), Ok(()));
        assert_eq!(eta.speed_modifier, Ok(29));
        assert_eq!(risk.combined_detection, Ok(Some(42)));
        assert_eq!(risk.coordination, Ok(Some(19)));
        assert!(risk.spotting_chance.is_err());
        assert_eq!(risk.night_spotting_chance, Ok(Some(0)));
    }
    assert_eq!(memory.calls(fault), 1);
}

#[test]
fn contribution_only_borrows_detection_stats_without_own_risk_later_leaves() {
    for own in [Ok(false), Err("original mission gate unreadable")] {
        let memory = mission_risk_fixture();
        let risk =
            mission_risk_reader(&memory).capture_mission_risk_union(TASK, 40, own, Ok(()), true);
        assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
        assert_eq!(risk.ship_count, Ok(Some(3)));
        let stats = risk.stats.unwrap().unwrap();
        assert!(stats.detection_138.is_ok() && stats.carrier_detection_160.is_ok());
        assert!(stats.visibility_148.is_err() && stats.visibility_150.is_err());
        assert_eq!(
            risk.coordination,
            own.map(|reached| if reached { Some(0) } else { None })
        );
        assert_eq!(risk.speed_base, risk.coordination);
        assert_mission_risk_graph_read_once(&memory);
        for offset in [0x148, 0x150] {
            assert_eq!(memory.calls(STATS + offset), 0);
        }
        assert_eq!(memory.observed(STATS + 8, 4), 1);
        assert_eq!(memory.observed(STATS + 0x1a8, 16), 1);
        assert_eq!(memory.observed(STATS + mission::STAT_RANGE, 8), 0);
        for key in [KEYS[0], 0x19, 0x55, 0x108, 0x19f] {
            assert_eq!(
                memory.calls(META + key as usize * metadata::STRIDE + metadata::FLAGS),
                0
            );
        }
        assert_eq!(memory.calls(TASK + ctask_force::CURRENT_FUEL_USAGE), 0);
        assert_mission_risk_skips_eta_fields(&memory);
    }
}

#[test]
fn original_counter_contributor_union_keeps_one_motion_and_modifier_graph() {
    let memory = motion_mission_risk_fixture(0);
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(41)
    .with_fresh_modifier_guard(Ok(()));
    let (mut count, mut owner) = (None, None);
    let risk = collector
        .visit_with_original_counter_contribution_observed(
            TASK,
            7,
            Ok(0),
            true,
            Ok(()),
            &mut count,
            &mut owner,
        )
        .unwrap();
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(risk.coordination, Ok(None));
    assert_eq!(risk.speed_base, Ok(None));
    assert_eq!(
        collector.finish(None).outcome,
        engine_native_memory::walk::Outcome::Rows { len: 1 }
    );
    assert_eq!(count, Some(Ok(0)));
    assert!(owner.unwrap().is_ok());
    assert_mission_risk_graph_read_once(&memory);
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
    assert_eq!(memory.calls(STATS + 0x138), 1);
    assert_eq!(memory.calls(STATS + 0x160), 1);
    assert_eq!(memory.calls(STATS + 0x148), 0);
    assert_eq!(memory.calls(STATS + 0x150), 0);
}

#[test]
fn borrowed_mission_risk_original_loan_error_precedes_clock_demand_and_guard() {
    for (hour, demand, guard) in [
        (None, Ok(true), Ok(())),
        (
            Some(35),
            Err("risk demand unreadable"),
            Err("risk guard mismatch"),
        ),
        (Some(35), Ok(false), Err("risk guard mismatch")),
    ] {
        let memory = motion_mission_risk_fixture(1);
        let mut rows = vec![];
        let mut paths = vec![];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            TERRAIN_IMAGE,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        );
        if let Some(hour) = hour {
            collector = collector.with_fresh_hour(hour);
        }
        let before = memory.reads.borrow().len();
        assert_eq!(
            collector.borrow_mission_risk(
                Err("original counter receiver identity invalid"),
                demand,
                guard,
            ),
            Err("original counter receiver identity invalid")
        );
        assert_eq!(memory.reads.borrow().len(), before);
        let walk = collector.finish(None);
        assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 0 });
        assert_eq!((walk.paths_len, walk.required_paths), (0, 0));
    }
}

#[test]
fn borrowed_mission_risk_authenticated_loan_without_hour_reads_no_graph() {
    let memory = motion_mission_risk_fixture(1);
    let mut rows = vec![];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    );
    let before = memory.reads.borrow().len();
    assert_eq!(
        collector.borrow_mission_risk(Ok((TASK, (71_u64 << 32) | 61)), Ok(true), Ok(()),),
        Err("fresh task-force risk sample hour unavailable")
    );
    assert_eq!(memory.reads.borrow().len(), before);
    let walk = collector.finish(None);
    assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 0 });
    assert_eq!((walk.paths_len, walk.required_paths), (0, 0));
}

#[test]
fn borrowed_mission_risk_unreached_demand_refusal_and_guard_leave_graph_and_census_untouched() {
    for (demand, guard, expected) in [
        (Ok(false), Ok(()), Ok(None)),
        (
            Err("original risk demand unreadable"),
            Ok(()),
            Err("original risk demand unreadable"),
        ),
        (
            Ok(true),
            Err("risk layout mismatch"),
            Err("risk layout mismatch"),
        ),
    ] {
        let memory = motion_mission_risk_fixture(1);
        let mut rows = vec![];
        let mut paths = vec![];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            TERRAIN_IMAGE,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        )
        .with_fresh_hour(35);
        let before = memory.reads.borrow().len();
        let risk = collector
            .borrow_mission_risk(Ok((TASK, (71_u64 << 32) | 61)), demand, guard)
            .unwrap();
        assert_eq!(risk.sample_hour, 35);
        assert_eq!(risk.combined_detection, expected);
        assert_eq!(memory.reads.borrow().len(), before);
        let walk = collector.finish(None);
        assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 0 });
        assert_eq!((walk.paths_len, walk.required_paths), (0, 0));
    }
}

#[test]
fn borrowed_mission_risk_preserves_existing_invalid_motion_roster_refusal() {
    let memory = mission_risk_fixture();
    let mut rows = vec![];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(35);
    let risk = collector
        .borrow_mission_risk(Ok((TASK, (71_u64 << 32) | 61)), Ok(true), Ok(()))
        .unwrap();
    let walk = collector.finish(None);
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(
        walk.outcome,
        engine_native_memory::walk::Outcome::Unavailable {
            reason: "task-force motion province roster invalid",
        }
    );
    assert_eq!((walk.paths_len, walk.required_paths), (0, 0));
    assert_mission_risk_graph_read_once(&memory);
    assert_mission_risk_skips_eta_fields(&memory);
    assert_eq!(memory.calls(TASK + cunit::LOGICAL_OWNER), 0);
    assert_eq!(memory.calls(TASK + cunit::ORDINARY_ROUTE_DATA), 0);
}

#[test]
fn borrowed_mission_risk_distinct_receiver_retains_normal_visit_metadata_failure() {
    const OUTSIDE_TASK: usize = 0x3c000;
    let risk_flags = META + keys::NAVAL_DETECTION as usize * metadata::STRIDE + metadata::FLAGS;
    for fault in [risk_flags, RISK_METADATA_ROOT] {
        let mut memory = motion_mission_risk_fixture(0);
        memory.u32(OUTSIDE_TASK + cunit::ROSTER_OWNER, 1);
        memory.fault = Some((fault, 4));
        memory.fail_once = true;
        let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
        let mut paths = vec![];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            TERRAIN_IMAGE,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        )
        .with_fresh_hour(35)
        .with_fresh_modifier_guard(Ok(()));
        let normal = collector
            .visit_with_mission_risk(TASK, 7, Ok(true), Ok(()))
            .unwrap();
        let outside = collector
            .borrow_mission_risk(Ok((OUTSIDE_TASK, (72_u64 << 32) | 61)), Ok(true), Ok(()))
            .unwrap();
        let walk = collector.finish(None);
        assert!(normal.combined_detection.is_err());
        assert_eq!(outside.combined_detection, normal.combined_detection);
        assert_eq!((normal.sample_hour, outside.sample_hour), (35, 35));
        assert_eq!(memory.calls(fault), 1);
        assert_eq!(memory.calls(RISK_METADATA_ROOT), 1);
        assert_eq!(memory.calls(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR), 2);
        assert_eq!(memory.calls(TASK + cunit::LOCATION), 1);
        assert_eq!(memory.calls(OUTSIDE_TASK + cunit::LOCATION), 1);
        assert_eq!(memory.calls(OUTSIDE_TASK + cunit::LOGICAL_OWNER), 0);
        assert_eq!(memory.calls(OUTSIDE_TASK + cunit::ORDINARY_ROUTE_DATA), 0);
        assert_eq!(memory.calls(OUTSIDE_TASK + cunit::CLASS), 0);
        assert_eq!(memory.calls(OUTSIDE_TASK + cunit::HANDLE), 0);
        assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
        assert_eq!((walk.paths_len, walk.required_paths), (0, 0));
        assert!(rows[0].fresh.is_some());
    }
}

#[test]
fn original_counter_risk_preserves_signed_count_and_owner_from_one_motion_attempt() {
    for (count, reached) in [(i32::MIN, true), (-1, true), (0, true), (1, false)] {
        let mut memory = motion_mission_risk_fixture(0);
        memory.u32(TASK + cunit::COMBAT_REFERENCE_COUNT, count as u32);
        let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
        let mut paths = vec![];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            TERRAIN_IMAGE,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        )
        .with_fresh_hour(36)
        .with_fresh_modifier_guard(Ok(()));
        let mut observed_count = None;
        let mut owner = None;
        let risk = collector
            .visit_with_original_counter_risk_observed(
                TASK,
                7,
                Ok(255),
                Ok(()),
                &mut observed_count,
                &mut owner,
            )
            .unwrap();
        let walk = collector.finish(None);
        assert_eq!(observed_count, Some(Ok(count)));
        assert_eq!(owner, Some(Ok(7)));
        assert_eq!(
            risk.combined_detection,
            Ok(reached.then_some(RISK_COMBINED))
        );
        assert_eq!(
            memory.calls(TASK + cunit::COMBAT_REFERENCE_DATA),
            1 + usize::from(count == 0)
        );
        assert_eq!(memory.calls(TASK + cunit::COMBAT_REFERENCE_COUNT), 0);
        assert_eq!(memory.calls(TASK + cunit::LOGICAL_OWNER), 1);
        if count == 0 {
            assert_eq!(walk.outcome, engine_native_memory::walk::Outcome::Rows { len: 1 });
        } else {
            assert_eq!(
                walk.outcome,
                engine_native_memory::walk::Outcome::Unavailable {
                    reason: "unit combat vector invalid"
                }
            );
        }
        if reached {
            assert_mission_risk_graph_read_once(&memory);
        } else {
            assert_eq!(memory.calls(RISK_METADATA_ROOT), 0);
        }
    }
}

#[test]
fn original_counter_risk_failed_combat_header_is_not_retried_as_a_readable_count_word() {
    let mut memory = motion_mission_risk_fixture(0);
    memory.fault = Some((TASK + cunit::COMBAT_REFERENCE_DATA, 16));
    memory.fail_once = true;
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(37)
    .with_fresh_modifier_guard(Ok(()));
    let mut count = None;
    let mut owner = None;
    let risk = collector
        .visit_with_original_counter_risk_observed(TASK, 7, Ok(1), Ok(()), &mut count, &mut owner)
        .unwrap();
    assert_eq!(count, Some(Err("unit combat vector invalid")));
    assert_eq!(owner, Some(Ok(7)));
    assert_eq!(risk.combined_detection, Err("unit combat vector invalid"));
    assert_eq!(
        collector.finish(None).outcome,
        engine_native_memory::walk::Outcome::Unavailable {
            reason: "unit combat vector invalid"
        }
    );
    assert_eq!(memory.calls(TASK + cunit::COMBAT_REFERENCE_DATA), 1);
    assert_eq!(memory.calls(TASK + cunit::COMBAT_REFERENCE_COUNT), 0);
    assert_eq!(memory.calls(RISK_METADATA_ROOT), 0);
    assert_mission_risk_skips_eta_fields(&memory);
}

#[test]
fn original_counter_risk_early_route_failure_reads_unattempted_count_once_after_native_flag_gate() {
    for (in_regions, expected, count_reads) in [
        (Ok(1), Ok(Some(RISK_COMBINED)), 1),
        (Ok(0), Ok(None), 0),
        (
            Err("original counter prefix failure"),
            Err("original counter prefix failure"),
            0,
        ),
    ] {
        let mut memory = motion_mission_risk_fixture(0);
        memory.fault = Some((TASK + cunit::ORDINARY_ROUTE_DATA, 16));
        let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
        let mut paths = vec![];
        let mut combats = cunit::FirstCombatMemo::new(1);
        let mut collector = ctask_force::MapMotionCollector::new(
            &memory,
            TERRAIN_IMAGE,
            STATE,
            &mut rows,
            &mut paths,
            &mut combats,
        )
        .with_fresh_hour(38)
        .with_fresh_modifier_guard(Ok(()));
        let mut count = None;
        let mut owner = None;
        let risk = collector
            .visit_with_original_counter_risk_observed(
                TASK,
                7,
                in_regions,
                Ok(()),
                &mut count,
                &mut owner,
            )
            .unwrap();
        assert_eq!(risk.combined_detection, expected);
        assert_eq!(count, (count_reads == 1).then_some(Ok(0)));
        assert_eq!(owner, Some(Ok(7)));
        assert_eq!(
            collector.finish(None).outcome,
            engine_native_memory::walk::Outcome::Unavailable {
                reason: "unit route invalid or longer than the province roster",
            }
        );
        assert_eq!(memory.calls(TASK + cunit::COMBAT_REFERENCE_DATA), 0);
        assert_eq!(
            memory.calls(TASK + cunit::COMBAT_REFERENCE_COUNT),
            count_reads
        );
        assert_eq!(memory.calls(TASK + cunit::LOGICAL_OWNER), 1);
        if count_reads == 1 {
            assert_mission_risk_graph_read_once(&memory);
        } else {
            assert_eq!(memory.calls(RISK_METADATA_ROOT), 0);
        }
        assert_mission_risk_skips_eta_fields(&memory);
    }
}

#[test]
fn original_counter_risk_failed_owner_is_retained_without_poisoning_independent_risk() {
    let mut memory = motion_mission_risk_fixture(0);
    memory.fault = Some((TASK + cunit::LOGICAL_OWNER, 4));
    memory.fail_once = true;
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(40)
    .with_fresh_modifier_guard(Ok(()));
    let mut count = None;
    let mut owner = None;
    let risk = collector
        .visit_with_original_counter_risk_observed(TASK, 7, Ok(1), Ok(()), &mut count, &mut owner)
        .unwrap();
    assert_eq!(owner, Some(Err("task-force logical owner unreadable")));
    assert_eq!(count, Some(Ok(0)));
    assert_eq!(risk.combined_detection, Ok(Some(RISK_COMBINED)));
    assert_eq!(
        collector.finish(None).outcome,
        engine_native_memory::walk::Outcome::Unavailable {
            reason: "task-force logical owner unreadable"
        }
    );
    assert_eq!(memory.calls(TASK + cunit::LOGICAL_OWNER), 1);
    assert_eq!(memory.calls(TASK + cunit::COMBAT_REFERENCE_DATA), 0);
    assert_eq!(memory.calls(TASK + cunit::COMBAT_REFERENCE_COUNT), 1);
    assert_mission_risk_graph_read_once(&memory);
    assert_mission_risk_skips_eta_fields(&memory);
}

#[test]
fn original_counter_risk_failed_shared_graph_never_falls_back_to_a_second_graph() {
    let mut memory = motion_mission_risk_fixture(0);
    memory.fault = Some((RISK_LOCATION, 8));
    memory.fail_once = true;
    let mut rows = vec![ctask_force::MapMotionRow::default(); 1];
    let mut paths = vec![];
    let mut combats = cunit::FirstCombatMemo::new(1);
    let mut collector = ctask_force::MapMotionCollector::new(
        &memory,
        TERRAIN_IMAGE,
        STATE,
        &mut rows,
        &mut paths,
        &mut combats,
    )
    .with_fresh_hour(41)
    .with_fresh_modifier_guard(Ok(()));
    let risk = collector
        .visit_with_original_counter_risk(TASK, 7, Ok(1), Ok(()))
        .unwrap();
    assert_eq!(
        risk.combined_detection,
        Err("fresh task-force operand unreadable")
    );
    assert_eq!(
        collector.finish(None).outcome,
        engine_native_memory::walk::Outcome::Rows { len: 1 }
    );
    assert_eq!(memory.calls(TASK + cunit::LOCATION), 1);
    assert_eq!(memory.calls(RISK_LOCATION), 1);
    assert_eq!(memory.calls(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR), 1);
    assert_eq!(memory.calls(TASK + cunit::STAT_BLOCK), 1);
    assert_eq!(memory.calls(STATE + crate::cgame_state::WEATHER_MANAGER), 0);
}
