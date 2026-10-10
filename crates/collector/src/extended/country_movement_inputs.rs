//! Bounded current-hour movement inputs shared by demanded country owners.
//!
//! The Ordinary job supplies the shared division census and already captured
//! railway-gun owner demands. Their native tooltip gates belong to those
//! source packs. A task force uses its fleet speed graph, not this transport
//! graph. Domain acquisition runs once here, followed by one query per demanded
//! canonical owner/key. No country-wide coefficient grid is acquired.

use super::production::Sources;
use crate::table_capture::{Arena, grow_capacity};
use engine::{
    Memory,
    carmy::{self, country_movement_inputs as native},
    ccountry,
    cpdx_array::Array,
    railway_gun_counter_inputs,
    walk::Outcome,
};
use replay::rows::{ControllerRiverSpeed, CountryMovementInputs, RegionTransportSpeed};

pub(crate) use native::LiveDemand;

const RIVER: u8 = 1;
const TRANSPORT: u8 = 2;

/// One demand occurrence from an independently authenticated moving unit.
/// Divisions can supply both graphs; railway guns supply transport only.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OwnerDemand {
    pub raw: u32,
    pub river: bool,
    pub transport: bool,
}

fn source_rows<'a, T>(
    sources: Sources<'a, T>,
    short: &'static str,
) -> (&'a [T], Option<&'static str>) {
    let Some(rows) = sources.rows.get(..sources.outcome.len()) else {
        return (&[], Some("country movement source length outside census"));
    };
    let reason = match sources.outcome {
        Outcome::Rows { .. } => None,
        Outcome::Truncated { .. } => Some(short),
        Outcome::Unavailable { reason } => Some(reason),
    };
    (rows, reason)
}

fn division_demands(
    seed: &carmy::DivisionSeed,
    base: usize,
) -> ([Option<OwnerDemand>; 2], Option<&'static str>) {
    let absent = [None; 2];
    let primary = base
        .checked_add(carmy::VTABLE_RVA)
        .map(|value| value as u64);
    if seed.pointer == 0
        || seed.division == 0
        || primary.is_none()
        || seed.primary_vtable != primary
        || seed.handle != Some(seed.division)
    {
        return (
            absent,
            Some("country movement division identity unobserved or changed"),
        );
    }
    match seed.class {
        Some(0) => {}
        Some(_) => return (absent, None),
        None => return (absent, Some("country movement division class unobserved")),
    }
    let Some(_route) = seed
        .ordinary_route
        .and_then(|header| Array::from_header(header, 4))
    else {
        return (
            absent,
            Some("country movement division route unobserved or invalid"),
        );
    };
    let land = seed.expeditionary_owner.and_then(|owner| {
        if (owner as i32) > 0 {
            Some(owner)
        } else {
            seed.roster_owner
        }
    });
    let river = land.map(|raw| OwnerDemand {
        raw,
        river: true,
        transport: false,
    });
    let transport = seed.logical_owner.map(|raw| OwnerDemand {
        raw,
        river: false,
        transport: true,
    });
    let failure = if land.is_none() {
        Some("country movement division land owner unobserved")
    } else if seed.logical_owner.is_none() {
        Some("country movement division logical owner unobserved")
    } else {
        None
    };
    ([river, transport], failure)
}

/// Uses only this acquisition's fixed seed fields and established gun rows.
/// Missing fields stay refused; neither graph nor failed seed fields are read.
pub(crate) fn owner_demands<'a>(
    base: usize,
    divisions: Sources<'a, carmy::DivisionSeed>,
    railway: Sources<'a, railway_gun_counter_inputs::Row>,
) -> (impl Iterator<Item = OwnerDemand> + 'a, Option<&'static str>) {
    let (divisions, division_failure) =
        source_rows(divisions, "country movement division census truncated");
    let (railway, railway_failure) =
        source_rows(railway, "country movement railway census truncated");
    let failure = division_failure
        .or(railway_failure)
        .or_else(|| {
            divisions
                .iter()
                .find_map(|seed| division_demands(seed, base).1)
        })
        .or_else(|| {
            railway
                .iter()
                .any(|row| row.gun == 0)
                .then_some("country movement railway identity unobserved")
        });
    let divisions = divisions
        .iter()
        .flat_map(move |seed| division_demands(seed, base).0.into_iter().flatten());
    let railway = railway
        .iter()
        .filter(|row| row.gun != 0)
        .map(|row| OwnerDemand {
            raw: row.logical_owner,
            river: false,
            transport: true,
        });
    (divisions.chain(railway), failure)
}

#[cfg(test)]
mod tests;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capacities {
    pub owners: usize,
    pub links: usize,
    pub controllers: usize,
    pub regions: usize,
    pub river: usize,
    pub transport: usize,
}

impl Capacities {
    pub(crate) fn raise_for_demand(&mut self, demand: LiveDemand) -> Result<(), &'static str> {
        for (capacity, required) in [
            (&mut self.owners, demand.country_slots),
            (&mut self.links, demand.raw_links),
            (&mut self.controllers, demand.controller_bound),
            (&mut self.regions, demand.region_bound),
        ] {
            if let Ok(required) = required {
                grow_capacity(capacity, required);
            }
        }
        let movers = demand.movers?;
        let mut failure = None;
        for (capacity, owners, keys) in [
            (
                &mut self.river,
                movers.land,
                demand
                    .controller_bound
                    .and_then(|bound| demand.country_slots.map(|slots| bound.min(slots))),
            ),
            (&mut self.transport, movers.logical, demand.region_bound),
        ] {
            let required = if owners == 0 {
                Ok(0)
            } else {
                keys.and_then(|keys| {
                    owners
                        .checked_mul(keys)
                        .ok_or("country movement demand overflow")
                })
            };
            match required {
                Ok(required) => grow_capacity(capacity, required),
                Err(reason) => {
                    failure.get_or_insert(reason);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Span {
    start: usize,
    len: usize,
}

#[derive(Debug, Default, Clone, Copy)]
struct Row {
    /// Native Country.id, exactly the enclosing CountryCounterInputs row key.
    country: u32,
    flags: u8,
    owner: Option<ccountry::Country>,
    river: Option<Span>,
    transport: Option<Span>,
    river_failure: Option<&'static str>,
    transport_failure: Option<&'static str>,
}

pub(crate) struct Captured {
    owners: Arena<Row>,
    raw_flags: Arena<u8>,
    controllers: Arena<u32>,
    regions: Arena<native::TransportRegion>,
    river: Arena<native::ControllerRiverSpeed>,
    transport: Arena<native::RegionTransportSpeed>,
    packet: native::CountryMovementPacket,
    pub(crate) source_failure: Option<&'static str>,
    pub(crate) controller_failure: Option<&'static str>,
    pub(crate) region_failure: Option<&'static str>,
    pub(crate) transport_graph_failure: Option<&'static str>,
    pub(crate) refused_raw_owners: usize,
    pub(crate) land_owners: usize,
    pub(crate) logical_owners: usize,
    pub(crate) controller_keys: usize,
    pub(crate) region_keys: usize,
}

impl Captured {
    pub(crate) fn with_capacities(capacity: Capacities) -> Self {
        let mut packet = native::CountryMovementPacket::try_new(capacity.owners)
            .unwrap_or_else(|_| native::CountryMovementPacket::vacant());
        let _ = packet.reserve_transport(capacity.transport, capacity.regions);
        Self {
            owners: Arena::new(capacity.owners),
            raw_flags: Arena::new(capacity.links),
            controllers: Arena::new(capacity.controllers),
            regions: Arena::new(capacity.regions),
            river: Arena::new(capacity.river),
            transport: Arena::new(capacity.transport),
            packet,
            source_failure: None,
            controller_failure: None,
            region_failure: None,
            transport_graph_failure: None,
            refused_raw_owners: 0,
            land_owners: 0,
            logical_owners: 0,
            controller_keys: 0,
            region_keys: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.owners.first.fill(Row::default());
        self.owners.reset();
        self.raw_flags.first.fill(0);
        self.raw_flags.reset();
        self.controllers.reset();
        self.regions.first.fill(native::TransportRegion::default());
        self.regions.reset();
        self.river.reset();
        self.transport.reset();
        self.packet.clear();
        self.source_failure = None;
        self.controller_failure = None;
        self.region_failure = None;
        self.transport_graph_failure = None;
        self.refused_raw_owners = 0;
        self.land_owners = 0;
        self.logical_owners = 0;
        self.controller_keys = 0;
        self.region_keys = 0;
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.owners.allocated_bytes()
            + self.raw_flags.allocated_bytes()
            + self.controllers.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.river.allocated_bytes()
            + self.transport.allocated_bytes()
            + self.packet.allocated_bytes()
    }

    /// Called only off acquisition threads. Each arena retains its full
    /// observed demand, including after a failed or short capture.
    pub(crate) fn grow(&mut self, capacity: &mut Capacities) {
        self.owners.grow(&mut capacity.owners);
        self.raw_flags.grow(&mut capacity.links);
        self.controllers.grow(&mut capacity.controllers);
        self.regions.grow(&mut capacity.regions);
        self.river.grow(&mut capacity.river);
        self.transport.grow(&mut capacity.transport);
        let _ = self
            .packet
            .reserve(capacity.owners)
            .and_then(|()| {
                self.packet
                    .reserve_transport(capacity.transport, capacity.regions)
            })
            .map_err(crate::allocation::io);
    }

    #[cfg(test)]
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        demands: impl IntoIterator<Item = OwnerDemand>,
        source_failure: Option<&'static str>,
    ) {
        self.capture_mode(memory, base, state, demands, source_failure, false);
    }

    pub(crate) fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        demands: impl IntoIterator<Item = OwnerDemand>,
        source_failure: Option<&'static str>,
    ) {
        self.capture_mode(memory, base, state, demands, source_failure, true);
    }

    fn capture_mode(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        demands: impl IntoIterator<Item = OwnerDemand>,
        source_failure: Option<&'static str>,
        growing: bool,
    ) {
        self.reset();
        self.source_failure = source_failure;
        let mut demands = demands
            .into_iter()
            .filter(|demand| demand.river || demand.transport)
            .peekable();
        if demands.peek().is_none() {
            return;
        }
        let countries = match ccountry::Reader::new(memory, base, state) {
            Ok(countries) => countries,
            Err(reason) => {
                self.source_failure.get_or_insert(reason);
                return;
            }
        };
        let country_slots = countries.count() as usize;
        let links = countries.arrays()[1].count as usize;
        self.owners.required = country_slots;
        self.raw_flags.required = links;
        if growing {
            crate::table_capture::grow_vec(&mut self.owners.first, country_slots);
            crate::table_capture::grow_vec(&mut self.raw_flags.first, links);
            let _ = self
                .packet
                .reserve(country_slots)
                .map_err(crate::allocation::io);
        }
        if self.owners.first.len() < country_slots || self.raw_flags.first.len() < links {
            // Without deduplication scratch, demand occurrences and the live
            // canonical slot count give conservative mover-owner bounds. This
            // count-only pass reads no owner graph and acquires no coefficients.
            let (mut land, mut logical) = (0_usize, 0_usize);
            for demand in demands {
                land = land.saturating_add(usize::from(demand.river));
                logical = logical.saturating_add(usize::from(demand.transport));
            }
            land = land.min(country_slots.saturating_sub(1));
            logical = logical.min(country_slots.saturating_sub(1));
            let domain = native::domain_demand(memory, state);
            if land != 0 {
                match domain.controllers {
                    Ok(required) => self.controllers.required = required,
                    Err(reason) => self.controller_failure = Some(reason),
                }
            }
            if logical != 0 {
                match domain.regions {
                    Ok(required) => self.regions.required = required,
                    Err(reason) => self.region_failure = Some(reason),
                }
            }
            self.coefficient_demand(
                land,
                self.controllers.required.min(country_slots),
                logical,
                self.regions.required,
            );
            self.source_failure
                .get_or_insert("country movement owner-demand arenas exhausted");
            return;
        }
        for demand in demands {
            let flags = (u8::from(demand.river) * RIVER) | (u8::from(demand.transport) * TRANSPORT);
            if flags == 0 {
                continue;
            }
            match self
                .raw_flags
                .first
                .get_mut(demand.raw as usize)
                .filter(|_| demand.raw != 0 && (demand.raw as usize) < links)
            {
                Some(slot) => *slot |= flags,
                None => {
                    self.refused_raw_owners = self.refused_raw_owners.saturating_add(1);
                    self.source_failure.get_or_insert(
                        "country movement demanded owner is zero or outside live links",
                    );
                }
            }
        }
        // Every distinct raw reference is resolved once. Alias occurrences
        // merge into one canonical owner, preserving its current native id.
        for raw in 1..links {
            let flags = self.raw_flags.first[raw];
            if flags == 0 {
                continue;
            }
            let owner = match countries.resolve(raw as u32) {
                Ok(owner) => owner,
                Err(reason) => {
                    self.refused_raw_owners = self.refused_raw_owners.saturating_add(1);
                    self.source_failure.get_or_insert(reason);
                    continue;
                }
            };
            let row = &mut self.owners.first[owner.canonical_slot as usize];
            row.country = owner.id;
            row.owner = Some(owner);
            row.flags |= flags;
        }
        self.land_owners = self.owners.first[..country_slots]
            .iter()
            .filter(|row| row.flags & RIVER != 0)
            .count();
        self.logical_owners = self.owners.first[..country_slots]
            .iter()
            .filter(|row| row.flags & TRANSPORT != 0)
            .count();
        if self.land_owners != 0 {
            let domain = if growing {
                native::capture_controller_domain_growing(
                    memory,
                    base,
                    state,
                    &countries,
                    &mut self.controllers.first,
                )
            } else {
                native::capture_controller_domain(
                    memory,
                    base,
                    state,
                    &countries,
                    &mut self.controllers.first,
                )
            };
            self.controllers.required = domain.required;
            match domain.len {
                Ok(len) => self.controllers.len = len,
                Err(reason) => self.controller_failure = Some(reason),
            }
        }
        if self.logical_owners != 0 {
            let domain = if growing {
                native::capture_region_domain_growing(memory, base, state, &mut self.regions.first)
            } else {
                native::capture_region_domain(memory, base, state, &mut self.regions.first)
            };
            self.regions.required = domain.required;
            match domain.len {
                Ok(len) => self.regions.len = len,
                Err(reason) => self.region_failure = Some(reason),
            }
        }
        self.controller_keys = self.controllers.len;
        self.region_keys = self.regions.len;
        self.coefficient_demand(
            self.land_owners,
            if self.controller_failure.is_some() {
                self.controllers.required.min(country_slots)
            } else {
                self.controller_keys
            },
            self.logical_owners,
            if self.region_failure.is_some() {
                self.regions.required
            } else {
                self.region_keys
            },
        );
        if growing {
            // Movement inputs consume the complete original keyed domains.
            crate::table_capture::grow_vec(&mut self.river.first, self.river.required);
            crate::table_capture::grow_vec(&mut self.transport.first, self.transport.required);
            let _ = self
                .packet
                .reserve_transport(self.transport.required, self.regions.len)
                .map_err(crate::allocation::io);
        }
        for row in &mut self.owners.first[..country_slots] {
            let Some(owner) = row.owner.take() else {
                continue;
            };
            if row.flags & RIVER != 0 {
                if let Some(reason) = self.controller_failure {
                    row.river_failure = Some(reason);
                } else {
                    let start = self.river.len;
                    let domain = native::capture_river(
                        memory,
                        base,
                        &countries,
                        owner,
                        &self.controllers.first[..self.controllers.len],
                        &mut self.river.first[start..],
                        &mut self.packet,
                    );
                    row.river_failure = domain.first_failure;
                    match domain.len {
                        Ok(len) => {
                            row.river = Some(Span { start, len });
                            self.river.len += len;
                        }
                        Err(reason) => row.river_failure = Some(reason),
                    }
                }
            }
            if row.flags & TRANSPORT != 0 {
                if let Some(reason) = self.region_failure {
                    row.transport_failure = Some(reason);
                } else {
                    let start = self.transport.len;
                    let domain = native::capture_transport(
                        memory,
                        base,
                        state,
                        owner,
                        &self.regions.first[..self.regions.len],
                        &mut self.transport.first[start..],
                        &mut self.packet,
                    );
                    match domain.len {
                        Ok(len) => {
                            row.transport = Some(Span { start, len });
                            self.transport.len += len;
                        }
                        Err(reason) => row.transport_failure = Some(reason),
                    }
                }
            }
        }
        self.transport_graph_failure = self.packet.transport_failure();
        self.packet.clear();
        self.regions.first.fill(native::TransportRegion::default());
        self.regions.len = 0;
        // Compact only output rows, now stripped of all native pointers.
        let mut written = 0;
        for index in 0..country_slots {
            if self.owners.first[index].flags != 0 {
                self.owners.first[written] = self.owners.first[index];
                written += 1;
            }
        }
        self.owners.len = written;
        self.owners.first[..written].sort_unstable_by_key(|row| row.country);
    }

    /// Off-thread projection joins by authenticated native Country.id, not the
    /// optional counter-prefix row index. An undemanded owner has absent lists.
    pub(crate) fn for_country(&self, country: u32) -> std::io::Result<CountryMovementInputs> {
        let rows = &self.owners.first[..self.owners.len];
        let Ok(index) = rows.binary_search_by_key(&country, |row| row.country) else {
            return Ok(CountryMovementInputs::default());
        };
        let row = &rows[index];
        let river = row
            .river
            .map(|span| {
                let values = self
                    .river
                    .first
                    .get(..self.river.len)
                    .and_then(|values| values.get(span.start..span.start.checked_add(span.len)?))
                    .ok_or_else(|| {
                        std::io::Error::other("country movement river span outside acquired arena")
                    })?;
                engine::allocation::collect_vec(values.iter().map(|value| ControllerRiverSpeed {
                    target_slot: value.target_slot,
                    modifier: value.modifier,
                }))
            })
            .transpose()?;
        let transport = row
            .transport
            .map(|span| {
                let values = self
                    .transport
                    .first
                    .get(..self.transport.len)
                    .and_then(|values| values.get(span.start..span.start.checked_add(span.len)?))
                    .ok_or_else(|| {
                        std::io::Error::other(
                            "country movement transport span outside acquired arena",
                        )
                    })?;
                engine::allocation::collect_vec(values.iter().map(|value| RegionTransportSpeed {
                    region: value.region,
                    speed: value.speed,
                }))
            })
            .transpose()?;
        Ok(CountryMovementInputs {
            river,
            transport,
            ..Default::default()
        })
    }

    fn coefficient_demand(
        &mut self,
        land: usize,
        controllers: usize,
        logical: usize,
        regions: usize,
    ) {
        match native::Demand::for_owners(land, controllers, logical, regions) {
            Ok(required) => {
                self.river.required = required.river;
                self.transport.required = required.transport;
            }
            Err(reason) => {
                self.controller_failure.get_or_insert(reason);
                self.region_failure.get_or_insert(reason);
            }
        }
    }

    /// Named independent query refusals for the existing off-thread status work.
    /// The middle value identifies the independent field whose query refused.
    pub(crate) fn refusals(&self) -> impl Iterator<Item = (u32, &'static str, &'static str)> + '_ {
        self.owners.first[..self.owners.len].iter().flat_map(|row| {
            let owner = [
                ("river", row.river_failure),
                ("transport", row.transport_failure),
            ]
            .into_iter()
            .filter_map(move |(field, reason)| reason.map(|reason| (row.country, field, reason)));
            let regions = row
                .transport
                .into_iter()
                .flat_map(|span| self.transport.first[span.start..span.start + span.len].iter())
                .filter_map(move |region| {
                    region
                        .failure
                        .map(|reason| (row.country, "transport", reason))
                });
            owner.chain(regions)
        })
    }
}

#[derive(Default)]
pub(crate) struct Work {
    owners_captured: u64,
    refused_raw_owners: u64,
    refusals: Vec<(&'static str, u64)>,
    required: Capacities,
    capacity: Capacities,
}

impl Work {
    pub(crate) fn observe(&mut self, capture: &Captured) -> std::io::Result<()> {
        self.owners_captured = self
            .owners_captured
            .saturating_add(capture.owners.len as u64);
        self.refused_raw_owners = self
            .refused_raw_owners
            .saturating_add(capture.refused_raw_owners as u64);
        let domain_failures = [
            ("river", capture.controller_failure),
            ("transport", capture.region_failure),
        ]
        .into_iter()
        .filter_map(|(field, reason)| {
            reason.filter(|_| {
                !capture
                    .refusals()
                    .any(|(_, recorded_field, _)| recorded_field == field)
            })
        });
        for reason in capture
            .source_failure
            .into_iter()
            .chain(domain_failures)
            .chain(capture.refusals().map(|(_, _, reason)| reason))
        {
            let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
                Ok(index) => index,
                Err(index) => {
                    engine::allocation::reserve(&mut self.refusals, 1)?;
                    self.refusals.insert(index, (reason, 0));
                    index
                }
            };
            self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
        }
        for (required, observed) in [
            (&mut self.required.owners, capture.owners.required),
            (&mut self.required.links, capture.raw_flags.required),
            (&mut self.required.controllers, capture.controllers.required),
            (&mut self.required.regions, capture.regions.required),
            (&mut self.required.river, capture.river.required),
            (&mut self.required.transport, capture.transport.required),
        ] {
            *required = (*required).max(observed);
        }
        self.capacity = Capacities {
            owners: capture.owners.first.len(),
            links: capture.raw_flags.first.len(),
            controllers: capture.controllers.first.len(),
            regions: capture.regions.first.len(),
            river: capture.river.first.len(),
            transport: capture.transport.first.len(),
        };
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "owners_captured": self.owners_captured,
            "refused_raw_owners": self.refused_raw_owners,
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "owner_slots_required": self.required.owners,
            "raw_links_required": self.required.links,
            "controller_scratch_required": self.required.controllers,
            "region_scratch_required": self.required.regions,
            "river_required": self.required.river,
            "transport_required": self.required.transport,
            "river_capacity": self.capacity.river,
            "transport_capacity": self.capacity.transport,
            "transport_failure_scope": "independent owner/region queries; first shared-control and operand refusals are retained for the acquisition",
        })
    }
}
