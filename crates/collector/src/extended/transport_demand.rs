//! Off-thread live transport hints. No hint is an acquired value or admission
//! certificate; failed families preserve storage and remain named/counted.
use crate::country_scale::DemandFailures;
use engine::{Memory, ccountry, cpdx_array::Array, read};

#[derive(Default)]
pub(crate) struct Demand {
    pub missions: super::transport_mission_shared::Capacities,
    pub convoys: super::transport_convoys::Capacities,
    pub spotting: super::transport_spotting_context::Capacities,
    pub air: [usize; 5],
    pub refusal: Option<&'static str>,
}

static FAILURES: [DemandFailures; 9] = [const { DemandFailures::new() }; 9];
const NAMES: [&str; 9] = [
    "convoys",
    "naval_ledgers",
    "naval_search",
    "spotting_owners",
    "air",
    "risk_ships",
    "air_modifier_entries",
    "weather_modifier_entries",
    "world",
];
pub(crate) fn status() -> impl serde::Serialize {
    crate::status::map(
        NAMES
            .into_iter()
            .zip(FAILURES.iter().map(DemandFailures::diagnostics)),
    )
}

fn add(total: &mut usize, count: usize) -> Result<(), &'static str> {
    *total = total
        .checked_add(count)
        .ok_or("transport sizing demand overflow")?;
    Ok(())
}
fn array(
    memory: &impl Memory,
    root: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    root.checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .ok_or("transport sizing header unreadable or invalid")
}

pub(crate) fn observe(memory: &impl Memory, base: usize) -> Result<Option<Demand>, &'static str> {
    let Some(state) = FAILURES[8].observe(crate::country_scale::demand_game_state(memory, base))?
    else {
        return Ok(None);
    };
    let mut demand = Demand::default();
    let mut accept = |index: usize, result: Result<(), &'static str>| {
        if let Err(reason) = FAILURES[index].observe(result) {
            demand.refusal.get_or_insert(reason);
        }
    };
    accept(
        0,
        convoys(memory, base, state).map(|value| demand.convoys = value),
    );
    accept(
        1,
        ledgers(memory, base, state).map(|value| demand.missions = value),
    );
    accept(
        2,
        engine::transport_naval_search::demand(memory, base, state)
            .map(|value| demand.missions.search = value),
    );
    accept(
        3,
        spotting(memory, base, state).and_then(|(value, failure)| {
            demand.spotting = value;
            failure.map_or(Ok(()), Err)
        }),
    );
    accept(
        4,
        air(memory, base, state).and_then(|(value, failure)| {
            demand.air[..4].copy_from_slice(&value);
            failure.map_or(Ok(()), Err)
        }),
    );
    accept(
        5,
        ships(memory, base, state).map(|value| demand.missions.ships = value),
    );
    accept(
        6,
        engine::transport_air_normalization::wing_entry_demand(memory, base, state)
            .map(|count| demand.air[4] = count),
    );
    accept(
        7,
        engine::transport_air_normalization::weather_entry_demand(memory, base, state)
            .map(|count| demand.missions.weather_entries = count),
    );
    Ok(Some(demand))
}

fn convoys(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<super::transport_convoys::Capacities, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut demand = super::transport_convoys::Capacities {
        countries: countries.count() as usize,
        ..Default::default()
    };
    for slot in 1..countries.count() {
        if let Some(country) = countries.at(slot)? {
            let (pool, reservations) = engine::cconvoys::transport::demand(memory, base, country)?;
            add(&mut demand.pool, pool)?;
            add(&mut demand.reservations, reservations)?;
        }
    }
    demand.variants = demand.pool;
    Ok(demand)
}

fn ledgers(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<super::transport_mission_shared::Capacities, &'static str> {
    let reader = engine::country_naval_movement_inputs::Reader::new(memory, base, state);
    let mut demand = super::transport_mission_shared::Capacities {
        navies: reader.bounds().navy_slots?,
        ..Default::default()
    };
    for slot in 0..demand.navies {
        let mut navy = reader.observe_navy(
            u32::try_from(slot).map_err(|_| "transport navy sizing slot overflow")?,
        )?;
        add(
            &mut demand.access_modes,
            navy.take_access_modes()?.count as usize,
        )?;
        engine::naval_mission_inputs::ledger_regions_in(
            memory,
            navy.take_ledger_regions()?,
            |_, ledger| {
                add(&mut demand.ledgers, 1)?;
                add(&mut demand.occurrences, ledger.count as usize)
            },
        )?;
    }
    // Every distinct nullable receiver has at least one original registration.
    demand.receivers = demand.occurrences;
    Ok(demand)
}

fn spotting(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<
    (
        super::transport_spotting_context::Capacities,
        Option<&'static str>,
    ),
    &'static str,
> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut demand = super::transport_spotting_context::Capacities {
        owners: countries.count() as usize,
        ..Default::default()
    };
    let mut failure = None;
    for slot in 0..countries.count() {
        let country = if slot == 0 {
            Some(countries.reserved()?)
        } else {
            countries.at(slot)?
        };
        let Some(country) = country else {
            continue;
        };
        let owner = engine::transport_spotting_context::Owner::capture_authenticated(
            memory, base, &countries, country,
        );
        // The original packet keeps the two arrays independently, including
        // inactive owners. Preserve each readable header's storage demand.
        for result in [
            owner.radar.and_then(|radar| {
                add(&mut demand.radar_values, radar.signed_count.max(0) as usize)?;
                radar.data.map(|_| ())
            }),
            owner.intel.and_then(|intel| intel.array).and_then(|intel| {
                add(&mut demand.intel_values, intel.signed_count.max(0) as usize)?;
                intel.data.map(|_| ())
            }),
        ] {
            if let Err(reason) = result {
                failure.get_or_insert(reason);
            }
        }
    }
    Ok((demand, failure))
}

fn air(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<([usize; 4], Option<&'static str>), &'static str> {
    let mut demand = [0; 4];
    let mut countries = None;
    let mut failure = None;
    let result = engine::cstrategic_air::visit_with_slots(
        memory,
        base,
        state,
        |root| {
            let regions = (|| {
                // GetRegionalAirWingArray@140C44FA0 and RegisterMission@140C4E360
                // index +0x98's nested headers at 0x18, not their 16-byte payload.
                let stride = engine::cpdx_array::STRIDE;
                let outer = array(
                    memory,
                    root.address,
                    engine::cstrategic_air::REGIONAL_MISSIONS,
                    stride,
                )?;
                add(&mut demand[1], outer.count as usize)?;
                const BATCH: usize = 128;
                let mut headers = [0_u8; BATCH * 16];
                let fields = [engine::memory::RecordField {
                    source_offset: 0,
                    destination_offset: 0,
                    len: 16,
                }];
                let layout = engine::memory::RecordLayout::new(&fields, 16)
                    .ok_or("transport air sizing header layout invalid")?;
                for first in (0..outer.count).step_by(BATCH) {
                    let count = ((outer.count - first) as usize).min(BATCH);
                    let at = outer
                        .element(first, stride)
                        .ok_or("transport air sizing region overflow")?;
                    let copied = memory.copy_records(
                        engine::memory::RecordSources::Strided {
                            base: at,
                            stride,
                            count,
                        },
                        &layout,
                        &mut headers[..count * 16],
                    );
                    for bytes in headers[..copied.min(count) * 16].chunks_exact(16) {
                        let header = Array::from_header(bytes.try_into().unwrap(), 8)
                            .ok_or("transport air sizing regional header invalid")?;
                        add(&mut demand[2], header.count as usize)?;
                    }
                    if copied < count {
                        return Err("transport air sizing regional headers unreadable");
                    }
                }
                Ok(())
            })();
            // 140F96470 consumes original +E0 factors with strideA0, word+80.
            let factors = array(memory, root.address, 0xe0, 0xa0)
                .and_then(|factors| add(&mut demand[3], factors.count as usize));
            for result in [regions, factors] {
                if let Err(reason) = result {
                    failure.get_or_insert(reason);
                }
            }
            Ok(())
        },
        |observation| {
            if let engine::cstrategic_air::SlotObservation::Domain(count) = observation {
                countries = Some(count as usize);
            }
        },
    );
    if let Err(reason) = result {
        failure.get_or_insert(reason);
    }
    demand[0] =
        countries.ok_or_else(|| failure.unwrap_or("transport air sizing domain missing"))?;
    Ok((demand, failure))
}

fn ships(memory: &impl Memory, base: usize, state: usize) -> Result<usize, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut models = 0;
    engine::cfleet::visit_task_forces(memory, base, &countries, |visit| {
        if let engine::cfleet::Visit::TaskForce(task, _) = visit {
            add(
                &mut models,
                array(memory, task.task, engine::ctask_force::SHIPS, 8)?.count as usize,
            )?;
        }
        Ok(())
    })?;
    let reader = engine::country_naval_movement_inputs::Reader::new(memory, base, state);
    let mut risk = 0;
    for slot in 0..reader.bounds().navy_slots? {
        let mut navy = reader.observe_navy(
            u32::try_from(slot).map_err(|_| "transport navy sizing slot overflow")?,
        )?;
        engine::naval_mission_inputs::ledger_regions_in(
            memory,
            navy.take_ledger_regions()?,
            |_, ledger| {
                for slot in 0..ledger.count {
                    let mission = ledger
                        .element(slot, 8)
                        .and_then(|at| read::<u64>(memory, at))
                        .and_then(|pointer| usize::try_from(pointer).ok())
                        .ok_or("transport sizing mission pointer unreadable")?;
                    add(
                        &mut risk,
                        engine::transport_ledger_observation::ship_demand(memory, base, mission)?,
                    )?;
                }
                Ok(())
            },
        )?;
    }
    // The original census and nullable ledger receivers can contain disjoint
    // SHIPS members. Their model journal retains the union, not its maximum.
    add(&mut models, risk)?;
    Ok(models)
}

#[cfg(test)]
#[path = "transport_demand_tests.rs"]
mod tests;
