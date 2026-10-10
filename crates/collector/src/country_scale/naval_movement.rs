//! Off-hour sizing for task-force names/supply/port operands, moving task forces and their naval dependencies.
//! Hints own no native pointers and never certify an hourly projection. Local
//! reserves are advisory: their failure cannot enter the terminal capture gate.

use crate::extended::{NavalMovementCapacities, NavalMovementSharedCapacities};
use engine::{Memory, ccountry, cfleet, country_naval_movement_inputs as native, cunit};

#[derive(Clone, Copy, Default)]
pub(crate) struct Demand {
    pub capacities: NavalMovementCapacities,
    pub shared: NavalMovementSharedCapacities,
    pub name_rows: usize,
    pub name_bytes: usize,
    /// Sum of independently valid origin headers, not a complete capture.
    pub supply_origins: usize,
    pub port: engine::ctask_force::port_mission::Demand,
    first_reason: Option<&'static str>,
}

impl Demand {
    pub(crate) fn refusal(&self) -> Option<&'static str> {
        self.first_reason
    }

    fn refuse(&mut self, reason: &'static str) {
        self.first_reason.get_or_insert(reason);
    }

    fn bounds(&mut self, bounds: native::Bounds) {
        match bounds.provinces {
            Ok(count) => self.capacities.provinces = self.capacities.provinces.max(count),
            Err(reason) => self.refuse(reason),
        }
        match bounds.navy_slots {
            Ok(count) => self.capacities.navies = self.capacities.navies.max(count),
            Err(reason) => self.refuse(reason),
        }
    }

    fn headers(&mut self, observed: &native::DemandedDemand) {
        let live = observed.live;
        self.capacities.owners = self.capacities.owners.max(observed.owners.required);
        self.capacities.navies = self.capacities.navies.max(observed.physical_slots.required);
        self.shared.nearest_rows = self
            .shared
            .nearest_rows
            .max(observed.physical_slots.required);
        self.capacities.foreign = self
            .capacities
            .foreign
            .max(live.navy.foreign_navy_slots.required);
        self.capacities.hubs = self.capacities.hubs.max(live.navy.supply_hubs.required);
        self.shared.nearest_bases = self
            .shared
            .nearest_bases
            .max(live.navy.nearest_bases.required);
        self.capacities.theatre_headers = self
            .capacities
            .theatre_headers
            .max(observed.theatre_header_scratch_required);
        match observed
            .owners
            .required
            .checked_add(live.navy.foreign_navy_slots.required)
        {
            Some(count) => {
                self.capacities.physical_slots = self.capacities.physical_slots.max(count);
            }
            None => self.refuse("naval movement sizing physical occurrence overflow"),
        }
    }

    fn names(&mut self, observed: &Owners) {
        self.name_rows = self.name_rows.max(observed.name_rows);
        self.name_bytes = self.name_bytes.max(observed.name_bytes);
        if let Some(reason) = observed.name_failure {
            self.refuse(reason);
        }
    }

    fn supply(&mut self, observed: &Owners) {
        self.supply_origins = self.supply_origins.max(observed.supply_origins);
        if let Some(reason) = observed.supply_failure {
            self.refuse(reason);
        }
    }

    fn port(&mut self, observed: &Owners) {
        self.port.searchers = self.port.searchers.max(observed.port.searchers);
        self.port.avoided = self.port.avoided.max(observed.port.avoided);
        self.port.bytes = self.port.bytes.max(observed.port.bytes);
        self.port.references = self.port.references.max(observed.port.references);
        if engine::ctask_force::port_mission::reference_slots(self.port.references).is_none() {
            self.port
                .refusal
                .get_or_insert("port reference sizing overflow");
            self.refuse("port reference sizing overflow");
        }
        if let Some(reason) = observed.port.refusal {
            self.port.refusal.get_or_insert(reason);
            self.refuse(reason);
        }
    }
}

fn scratch<T: Default>(count: usize) -> Result<Vec<T>, &'static str> {
    let mut values = Vec::new();
    grow_scratch(&mut values, count)?;
    Ok(values)
}

fn grow_scratch<T: Default>(values: &mut Vec<T>, count: usize) -> Result<(), &'static str> {
    if count > values.len() {
        values
            .try_reserve_exact(count - values.len())
            .map_err(|_| "naval movement sizing scratch allocation failed")?;
        values.resize_with(count, T::default);
    }
    Ok(())
}

struct Owners {
    // Indexed by canonical country slot, so repeated task forces add no owner.
    countries: Vec<Option<ccountry::Country>>,
    failure: Option<&'static str>,
    name_rows: usize,
    name_bytes: usize,
    name_failure: Option<&'static str>,
    supply_origins: usize,
    supply_failure: Option<&'static str>,
    port: engine::ctask_force::port_mission::Demand,
}

impl Owners {
    fn iter(&self) -> impl Iterator<Item = Result<ccountry::Country, &'static str>> + '_ {
        self.countries
            .iter()
            .flatten()
            .copied()
            .map(Ok)
            .chain(self.failure.map(Err))
    }

    fn complete_empty(&self) -> bool {
        self.failure.is_none() && self.countries.iter().all(Option::is_none)
    }
}

/// Match the fresh task-force packet's ordinary-route/target gates before
/// demanding its roster owner. Foreign navy keys never pass through here.
fn moving_owner<M: Memory>(
    memory: &M,
    countries: &ccountry::Reader<'_, M>,
    visit: cfleet::TaskForceVisit,
) -> Result<Option<ccountry::Country>, &'static str> {
    let field = |offset| {
        visit
            .task
            .checked_add(offset)
            .ok_or("naval movement sizing task-force address overflow")
    };
    let raw = engine::read::<u32>(memory, field(cunit::ROSTER_OWNER)?)
        .ok_or("naval movement sizing roster owner unreadable")?;
    let owner = countries.resolve(raw)?;
    if owner.pointer == 0 || owner.id == 0 || owner.canonical_slot == 0 || owner != visit.country {
        return Err("naval movement sizing task-force owner differs from fleet roster");
    }
    Ok(Some(owner))
}

fn owners<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
) -> Result<Owners, &'static str> {
    let mut observed = Owners {
        countries: scratch(countries.count() as usize)?,
        failure: None,
        name_rows: 0,
        name_bytes: 0,
        name_failure: None,
        supply_origins: 0,
        supply_failure: None,
        port: engine::ctask_force::port_mission::Demand::default(),
    };
    let walk = cfleet::visit_task_forces(memory, base, countries, |event| {
        let cfleet::Visit::TaskForce(visit, _) = event else {
            return Ok(());
        };
        // Names belong to every admitted task force, including stationary
        // ones. This existing authenticated visitor needs only string headers
        // and boundary probes; it neither copies payload nor adds a census.
        match observed.name_rows.checked_add(1) {
            Some(count) => observed.name_rows = count,
            None => {
                observed
                    .name_failure
                    .get_or_insert("task-force name sizing row overflow");
            }
        }
        match engine::ctask_force::cached_name_demand(memory, base, visit.task, visit.id) {
            Ok(bytes) => match observed.name_bytes.checked_add(bytes) {
                Some(count) => observed.name_bytes = count,
                None => {
                    observed
                        .name_failure
                        .get_or_insert("task-force name sizing byte overflow");
                }
            },
            Err(reason) => {
                observed.name_failure.get_or_insert(reason);
            }
        }
        // The same authenticated primary CUnit owns the supply header. A
        // failed optional header cannot erase names or moving-owner floors.
        match engine::cunit_supply_inputs::origin_demand(memory, visit.task) {
            Ok(count) => match observed.supply_origins.checked_add(count) {
                Some(count) => observed.supply_origins = count,
                None => {
                    observed
                        .supply_failure
                        .get_or_insert("task-force supply sizing origin overflow");
                }
            },
            Err(failure) => {
                observed.supply_failure.get_or_insert(failure.reason());
            }
        }
        match moving_owner(memory, countries, visit) {
            Ok(Some(owner)) => {
                let Some(entry) = observed.countries.get_mut(owner.canonical_slot as usize) else {
                    return Err("naval movement sizing owner outside country domain");
                };
                if entry.is_some_and(|known| known != owner) {
                    return Err("naval movement sizing canonical owner changed");
                }
                *entry = Some(owner);
            }
            Ok(None) => {}
            Err(reason) => {
                observed.failure.get_or_insert(reason);
            }
        }
        // These optional port demands use this same all-task-force visitor,
        // after the established name/supply/moving-owner observations. Nothing
        // allocates or follows a searcher here; only headers/string boundaries
        // size the independent prepared arenas.
        let port = engine::ctask_force::port_mission::demand(memory, base, visit.task, visit.kind);
        for (total, count) in [
            (&mut observed.port.searchers, port.searchers),
            (&mut observed.port.avoided, port.avoided),
            (&mut observed.port.bytes, port.bytes),
            (&mut observed.port.references, port.references),
        ] {
            if let Some(sum) = total.checked_add(count) {
                *total = sum;
            } else {
                *total = usize::MAX;
                observed
                    .port
                    .refusal
                    .get_or_insert("port sizing occurrence sum overflow");
            }
        }
        if let Some(reason) = port.refusal {
            observed.port.refusal.get_or_insert(reason);
        }
        Ok(())
    });
    if let Err(reason) = walk {
        observed.failure.get_or_insert(reason);
    }
    Ok(observed)
}

/// The first pass discovers foreign scratch, then one independently acquired
/// prepared pass completes a stable closure. Further live growth retains its
/// floors/refusal for the next poll. Native population remains uncapped; this
/// advisory work budget avoids chasing mutations between acquisition passes.
pub(crate) fn observe(
    memory: &impl Memory,
    base: usize,
    state: usize,
    foreign_floor: &mut usize,
) -> Demand {
    let mut demand = Demand::default();
    let mut slots = Vec::new();
    let mut foreign = Vec::new();
    for prepared in [false, true] {
        let reader = native::Reader::new(memory, base, state);
        let bounds = reader.bounds();
        if let Ok(count) = bounds.provinces {
            demand.capacities.provinces = demand.capacities.provinces.max(count);
        }
        let countries = match ccountry::Reader::new(memory, base, state) {
            Ok(countries) => countries,
            Err(reason) => {
                demand.bounds(bounds);
                demand.refuse(reason);
                break;
            }
        };
        demand.capacities.owner_slots = demand
            .capacities
            .owner_slots
            .max(countries.count() as usize);
        let owners = match owners(memory, base, &countries) {
            Ok(owners) => owners,
            Err(reason) => {
                demand.refuse(reason);
                break;
            }
        };
        demand.names(&owners);
        demand.supply(&owners);
        if let Some(reason) = owners.failure {
            demand.refuse(reason);
        }
        demand.port(&owners);
        if owners.complete_empty() {
            // The collector skips the reserved theatre on complete no demand.
            break;
        }
        let foreign_prepared = if prepared {
            true
        } else {
            match grow_scratch(&mut foreign, *foreign_floor) {
                Ok(()) => true,
                Err(reason) => {
                    demand.refuse(reason);
                    false
                }
            }
        };
        demand.bounds(bounds);
        let slot_count = (countries.count() as usize).max(bounds.navy_slots.unwrap_or(0));
        if let Err(reason) = grow_scratch(&mut slots, slot_count) {
            demand.refuse(reason);
            break;
        }
        let observed = reader.live_demand(&countries, owners.iter(), &mut slots, &mut foreign);
        *foreign_floor = (*foreign_floor).max(observed.foreign_scratch_required);
        demand.headers(&observed);
        // A native fault remains visible even if the next capacity-only pass
        // succeeds. The engine retains this separately from first scratch error.
        if let Some(reason) = observed.first_native_failure {
            demand.refuse(reason);
        }
        let short = slots.len() < observed.slot_scratch_required
            || foreign.len() < observed.foreign_scratch_required;
        if short && !prepared && foreign_prepared && observed.first_native_failure.is_none() {
            let grown = grow_scratch(&mut slots, observed.slot_scratch_required)
                .and_then(|()| grow_scratch(&mut foreign, observed.foreign_scratch_required));
            if let Err(reason) = grown {
                demand.refuse(reason);
                break;
            }
            continue;
        }
        for reason in [
            observed.owners.first_failure,
            observed.physical_slots.first_failure,
            observed.live.navy.nearest_bases.first_failure,
            observed.live.navy.foreign_navy_slots.first_failure,
            observed.live.navy.supply_hubs.first_failure,
            observed.live.theatres.first_failure,
            observed.live.theatre_areas.first_failure,
        ]
        .into_iter()
        .flatten()
        {
            demand.refuse(reason);
        }
        break;
    }
    // Theatre headers already observed for sizing above are not reacquired
    // within that pass. This fresh pass owns its domain/controls until return.
    if let Err(reason) = theatres(memory, base, state, &mut demand) {
        demand.refuse(reason);
    }
    demand
}

fn theatres<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    demand: &mut Demand,
) -> Result<(), &'static str> {
    let reader = native::Reader::new(memory, base, state);
    let bounds = reader.bounds();
    if let Ok(count) = bounds.provinces {
        demand.capacities.provinces = demand.capacities.provinces.max(count);
    }
    let countries = ccountry::Reader::new(memory, base, state)?;
    demand.capacities.owner_slots = demand
        .capacities
        .owner_slots
        .max(countries.count() as usize);
    let owners = owners(memory, base, &countries)?;
    demand.names(&owners);
    demand.supply(&owners);
    if let Some(reason) = owners.failure {
        demand.refuse(reason);
    }
    demand.port(&owners);
    let owner_count = owners.countries.iter().flatten().count();
    demand.capacities.owners = demand.capacities.owners.max(owner_count);
    let mut domain = scratch::<native::ControllerAreaProvince>(bounds.provinces.unwrap_or(0))?;
    let captured = reader.capture_controller_area_domain(&mut domain);
    demand.capacities.provinces = demand.capacities.provinces.max(captured.required);
    match captured
        .len
        .and_then(|len| reader.selected_area_edge_demand(&domain[..len]))
    {
        Ok(count) => demand.capacities.area_edges = demand.capacities.area_edges.max(count),
        Err(reason) => demand.refuse(reason),
    }
    if owners.complete_empty() {
        for province in &mut domain {
            province.clear();
        }
        return Ok(());
    }
    demand.bounds(bounds);
    let mut marks = scratch::<u8>(captured.required)?;
    let mut headers = scratch::<native::TheatreInput>(demand.capacities.theatre_headers)?;
    let mut required = 0_usize;
    for owner in owners.countries.iter().flatten().copied() {
        if let Some(count) = theatre(
            &reader,
            &countries,
            native::Owner::Positive(owner),
            &mut headers,
            captured.len.map(|len| &domain[..len]),
            &mut marks,
            demand,
        ) {
            required = required
                .checked_add(count)
                .ok_or("naval movement sizing theatre output overflow")?;
        }
    }
    demand.capacities.theatre_provinces = demand.capacities.theatre_provinces.max(required);
    match reader.reserved_owner() {
        Ok(owner) => {
            if let Some(count) = theatre(
                &reader,
                &countries,
                owner,
                &mut headers,
                captured.len.map(|len| &domain[..len]),
                &mut marks,
                demand,
            ) {
                demand.shared.reserved_provinces = demand.shared.reserved_provinces.max(count);
            }
        }
        Err(reason) => demand.refuse(reason),
    }
    for province in &mut domain {
        province.clear();
    }
    Ok(())
}

fn theatre<M: Memory>(
    reader: &native::Reader<'_, M>,
    countries: &ccountry::Reader<'_, M>,
    owner: native::Owner,
    headers: &mut [native::TheatreInput],
    domain: Result<&[native::ControllerAreaProvince], &'static str>,
    marks: &mut [u8],
    demand: &mut Demand,
) -> Option<usize> {
    let observed = reader.capture_theatre_headers(countries, owner, headers);
    demand.capacities.theatre_headers = demand
        .capacities
        .theatre_headers
        .max(observed.required_headers);
    if let Some(reason) = observed.areas.first_failure {
        demand.refuse(reason);
    }
    let projected = observed.len.and_then(|len| {
        domain.map(|domain| {
            reader.capture_theatre_membership(&mut headers[..len], domain, marks, &mut [])
        })
    });
    for header in headers {
        header.clear();
    }
    match projected {
        Ok(projected) => match projected.exact_province_count() {
            Ok(count) => Some(count),
            Err(reason) => {
                demand.refuse(reason);
                // Before a complete scan, required is a domain bound, not K.
                None
            }
        },
        Err(reason) => {
            demand.refuse(reason);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{
        cgame_state, cprovince, cstrategic_navy, cstrategic_navy_manager, ctask_force, ctheatre,
    };
    use std::cell::{Cell, RefCell};

    const STATE: usize = 0x1000;
    const COUNTRY: usize = 0x4500;
    const NAVY: usize = 0x7000;
    const TASK: usize = 0xd800;

    struct World {
        bytes: Vec<u8>,
        fault: Option<usize>,
        failed: Cell<bool>,
        reads: RefCell<Vec<(usize, usize)>>,
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if self
                .fault
                .is_some_and(|fault| address <= fault && fault < address + output.len())
                && !self.failed.replace(true)
            {
                return false;
            }
            let Some(source) = address
                .checked_add(output.len())
                .and_then(|end| self.bytes.get(address..end))
            else {
                return false;
            };
            output.copy_from_slice(source);
            true
        }
    }

    impl World {
        fn word(&mut self, at: usize, value: u32) {
            self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn pointer(&mut self, at: usize, value: usize) {
            self.bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }

        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.pointer(at, data);
            self.word(at + 8, count);
            self.word(at + 12, count);
        }

        fn pointers(&mut self, at: usize, values: &[usize]) {
            for (index, &value) in values.iter().enumerate() {
                self.pointer(at + index * 8, value);
            }
        }

        fn touches(&self, address: usize, len: usize) -> bool {
            self.reads
                .borrow()
                .iter()
                .any(|&(at, copied)| at < address + len && address < at + copied)
        }

        fn new() -> Self {
            let mut world = Self {
                bytes: vec![0; 0x10000],
                fault: None,
                failed: Cell::new(false),
                reads: RefCell::new(Vec::new()),
            };
            world.array(STATE + cgame_state::COUNTRIES, 0x3000, 4);
            world.array(STATE + cgame_state::COUNTRY_LINKS, 0x3100, 4);
            world.pointers(0x3000, &[0x4000, COUNTRY, 0x4800, COUNTRY]);
            for (slot, country, id, link) in [
                (0, 0x4000, 0, 0),
                (1, COUNTRY, 1, 1),
                (2, 0x4800, 2, 2),
                (3, COUNTRY, 1, 1),
            ] {
                world.pointer(country, ccountry::VTABLE_RVA);
                world.word(country + ccountry::ID, id);
                world.word(0x3100 + slot * 4, link);
            }
            world.array(COUNTRY + ccountry::FLEETS, 0x3300, 1);
            world.pointers(0x3300, &[0xd000]);
            world.pointer(0xd000, cfleet::VTABLE_RVA);
            world.pointer(
                0xd000 + cfleet::HANDLE,
                (1 << 32) | cfleet::HANDLE_TYPE as usize,
            );
            world.pointer(0xd000 + cfleet::OWNER, COUNTRY);
            world.array(0xd000 + cfleet::TASK_FORCES, 0x3380, 2);
            world.pointers(0x3380, &[TASK, 0xdc00]);
            for (id, task) in [(1, TASK), (2, 0xdc00)] {
                world.pointer(task, ctask_force::VTABLE_RVA);
                world.pointer(
                    task + ctask_force::SECONDARY_BASE,
                    ctask_force::SECONDARY_VTABLE_RVA,
                );
                world.pointer(
                    task + cunit::HANDLE,
                    (id << 32) | ctask_force::DEFAULT_HANDLE_TYPE as usize,
                );
                world.pointer(task + ctask_force::FLEET, 0xd000);
                // Authenticated retained CUnit name: genuine empty inline text.
                world.pointer(task + 0x258 + 24, 15);
                world.word(task + cunit::ROSTER_OWNER, 3);
                world.array(task + cunit::ORDINARY_ROUTE_DATA, 0xf000, 1);
            }
            world.pointer(STATE + cgame_state::STRATEGIC_NAVY_MANAGER, 0x6000);
            world.pointer(0x6000, cstrategic_navy_manager::VTABLE_RVA);
            world.array(0x6000 + cstrategic_navy_manager::NAVIES, 0x6100, 4);
            world.pointers(0x6100, &[0, NAVY, 0x7200, 0x7400]);
            for navy in [NAVY, 0x7200, 0x7400] {
                world.pointer(navy, cstrategic_navy::VTABLE_RVA);
                world.pointer(
                    navy + cstrategic_navy::SECONDARY,
                    cstrategic_navy::SECONDARY_VTABLE_RVA,
                );
                world.pointer(navy + native::MANAGER, 0x6000);
            }
            world.array(NAVY + native::NEAREST_BASES, 0x8000, 5);
            // A dormant manager slot with a large cache is outside the closure.
            world.array(0x7200 + native::NEAREST_BASES, 0x100000, 20000);
            // Slot3 aliases country1 but is its own physical navy dependency.
            world.array(0x7400 + native::NEAREST_BASES, 0x8400, 7);
            world.array(NAVY + native::FOREIGN_NAVY_SLOTS, 0x8200, 3);
            for (index, value) in [3, 0, 3].into_iter().enumerate() {
                world.word(0x8200 + index * 4, value);
            }
            world.array(NAVY + native::SUPPLY_HUBS, 0x8300, 3);
            world.array(STATE + cgame_state::PROVINCES, 0x3200, 5);
            world.pointers(0x3200, &[0, 0xa000, 0xa200, 0xa400, 0xa600]);
            for (id, province, area) in [
                (1, 0xa000, 0xe100),
                (2, 0xa200, 0xe200),
                (3, 0xa400, 0),
                (4, 0xa600, 0xe300),
            ] {
                world.pointer(province, cprovince::VTABLE_RVA);
                world.word(province + cprovince::ID, id);
                world.pointer(province + cprovince::CONTROLLER_AREA, area);
                if area != 0 {
                    world.pointer(area, engine::ccontroller_area::VTABLE_RVA);
                }
            }
            for (country, roster, theatre, owner, id, areas, values) in [
                (COUNTRY, 0xb000, 0xb100, 3, 1, 0xb600, &[0xe100, 0][..]),
                (0x4000, 0xb800, 0xb900, 0, 2, 0xbc00, &[0xe200][..]),
            ] {
                world.array(country + ccountry::THEATRES, roster, 1);
                world.pointers(roster, &[theatre]);
                world.pointer(theatre, ctheatre::VTABLE_RVA);
                world.pointer(
                    theatre + ctheatre::HANDLE,
                    (id << 32) | ctheatre::REFERENCE_TYPE as usize,
                );
                world.word(theatre + ctheatre::OWNER, owner);
                world.array(
                    theatre + ctheatre::CONTROLLER_AREA_DATA,
                    areas,
                    values.len() as u32,
                );
                world.pointers(areas, values);
            }
            world
        }
    }

    #[test]
    fn attach_hint_sizes_foreign_closure_and_exact_theatres_without_nearest_payloads() {
        let world = World::new();
        let mut foreign_floor = 0;
        let demand = observe(&world, 0, STATE, &mut foreign_floor);
        assert_eq!(demand.refusal(), None);
        assert_eq!(demand.capacities.owners, 1);
        assert_eq!(demand.capacities.owner_slots, 4);
        assert_eq!(demand.capacities.physical_slots, 4);
        // The original physical manager's four slots include reserved zero.
        assert_eq!(demand.capacities.navies, 4);
        assert_eq!(demand.capacities.foreign, 3);
        assert_eq!(foreign_floor, 3);
        assert_eq!(demand.capacities.hubs, 3);
        assert_eq!(demand.shared.nearest_rows, 3);
        assert_eq!(demand.shared.nearest_bases, 12);
        assert_eq!(demand.capacities.provinces, 4);
        assert_eq!(demand.capacities.theatre_headers, 1);
        assert_eq!(demand.capacities.theatre_provinces, 2);
        assert_eq!(demand.shared.reserved_provinces, 1);
        assert!(!world.touches(0x8000, 40));
        assert!(!world.touches(0x8400, 56));
        assert!(!world.touches(0x8300, 12));
        assert!(!world.touches(0x7200, 0x90));
        world.reads.borrow_mut().clear();
        let repeated = observe(&world, 0, STATE, &mut foreign_floor);
        assert_eq!(repeated.refusal(), None);
        assert_eq!(repeated.shared.nearest_bases, 12);
        // The owner census does not reach ordinary-route headers; its demand
        // comes from authenticated task-force identities and roster owners.
        assert_eq!(
            world
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, len)| at == TASK + cunit::ORDINARY_ROUTE_DATA && len == 16)
                .count(),
            0
        );
    }

    #[test]
    fn complete_empty_task_force_census_skips_reserved_theatre_and_navy_payloads() {
        let mut world = World::new();
        // No authenticated TF receiver demands a movement owner. An empty
        // fleet, unlike a stationary task force, proves the no-demand path.
        world.array(0xd000 + cfleet::TASK_FORCES, 0, 0);
        let mut foreign_floor = usize::MAX;
        let demand = observe(&world, 0, STATE, &mut foreign_floor);
        assert_eq!(demand.refusal(), None);
        assert_eq!(foreign_floor, usize::MAX);
        assert_eq!(demand.capacities.owners, 0);
        assert_eq!(demand.capacities.theatre_headers, 0);
        assert_eq!(demand.shared.reserved_provinces, 0);
        assert_eq!(demand.name_rows, 0);
        assert!(!world.touches(0xb800, 8));
        assert!(!world.touches(NAVY, 0x90));
    }

    #[test]
    fn first_hour_area_edges_size_from_unique_roots_even_without_task_forces() {
        let mut world = World::new();
        world.array(0xd000 + cfleet::TASK_FORCES, 0, 0);
        // Two original provinces select the same area; count its header once.
        world.pointer(0xa200 + cprovince::CONTROLLER_AREA, 0xe100);
        world.word(0xe100 + 0xa0, 13);
        world.word(0xe300 + 0xa0, 7);
        let mut floor = 0;
        let demand = observe(&world, 0, STATE, &mut floor);
        assert_eq!(demand.refusal(), None);
        assert_eq!(demand.capacities.area_edges, 20);
        assert_eq!(demand.capacities.owners, 0);
        assert!(!world.touches(0xe100 + 0x90, 8));
        assert!(!world.touches(0xb800, 8));
        assert!(!world.touches(NAVY, 0x90));
        world.word(0xe100 + 0xa0, u32::MAX);
        let demand = observe(&world, 0, STATE, &mut floor);
        assert_eq!(
            demand.refusal(),
            Some("selected area sizing contact count unreadable or negative")
        );
    }

    #[test]
    fn idle_task_forces_demand_bounded_owner_closure_without_route_or_cache_payloads() {
        let mut world = World::new();
        for task in [TASK, 0xdc00] {
            world.array(task + cunit::ORDINARY_ROUTE_DATA, 0, 0);
        }
        // A retained target, like an empty ordinary route, is no demand veto.
        world.pointer(0xdc00 + cunit::MOVEMENT_TARGET, 0xa000);
        let mut floor = 0;
        let demand = observe(&world, 0, STATE, &mut floor);
        assert_eq!(demand.refusal(), None);
        assert_eq!(demand.capacities.owners, 1);
        assert_eq!(demand.capacities.owner_slots, 4);
        assert_eq!(demand.capacities.physical_slots, 4);
        assert_eq!(demand.capacities.navies, 4);
        assert_eq!(demand.capacities.foreign, 3);
        assert_eq!(floor, 3);
        assert_eq!(demand.shared.nearest_bases, 12);
        assert_eq!(demand.capacities.theatre_headers, 1);
        assert_eq!(demand.capacities.theatre_provinces, 2);
        assert_eq!(demand.shared.reserved_provinces, 1);
        assert!(world.touches(TASK + cunit::ROSTER_OWNER, 4));
        assert!(world.touches(NAVY, 0x90));
        for task in [TASK, 0xdc00] {
            assert!(!world.touches(task + cunit::ORDINARY_ROUTE_DATA, 16));
            assert!(!world.touches(task + cunit::MOVEMENT_TARGET, 8));
        }
        assert!(!world.touches(0x8000, 40));
        assert!(!world.touches(0x8400, 56));
        assert!(!world.touches(0x8300, 12));
        // The unrelated physical manager slot remains outside this closure.
        assert!(!world.touches(0x7200, 0x90));
    }

    #[test]
    fn failed_idle_roster_owner_is_not_retried_inside_the_owner_census() {
        let mut world = World::new();
        for task in [TASK, 0xdc00] {
            world.array(task + cunit::ORDINARY_ROUTE_DATA, 0, 0);
        }
        world.fault = Some(TASK + cunit::ROSTER_OWNER);
        let countries = ccountry::Reader::new(&world, 0, STATE).unwrap();
        let observed = owners(&world, 0, &countries).unwrap();
        assert_eq!(
            observed.failure,
            Some("naval movement sizing roster owner unreadable")
        );
        // The other valid TF still supplies its owner floor, but cannot erase
        // the earlier failed observation in this census.
        assert_eq!(observed.countries.iter().flatten().count(), 1);
        assert_eq!(observed.name_rows, 2);
        assert_eq!(
            world
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, len)| at == TASK + cunit::ROSTER_OWNER && len == 4)
                .count(),
            1
        );
        assert!(!world.touches(TASK + cunit::ORDINARY_ROUTE_DATA, 16));
    }

    #[test]
    fn name_hint_includes_stationary_task_forces_without_copying_heap_payloads() {
        let mut world = World::new();
        world.word(TASK + cunit::ORDINARY_ROUTE_COUNT, 0);
        world.pointer(0xdc00 + cunit::MOVEMENT_TARGET, 0xa000);
        let text = b"A long retained task-force name";
        world.pointer(TASK + 0x258, 0xf400);
        world.pointer(TASK + 0x258 + 16, text.len());
        world.pointer(TASK + 0x258 + 24, text.len());
        world.bytes[0xf400..0xf400 + text.len()].copy_from_slice(text);
        world.bytes[0xdc00 + 0x258..0xdc00 + 0x258 + 6].copy_from_slice(b"Stored");
        world.pointer(0xdc00 + 0x258 + 16, 6);
        let mut floor = 0;
        let demand = observe(&world, 0, STATE, &mut floor);
        assert_eq!(demand.refusal(), None);
        assert_eq!((demand.name_rows, demand.name_bytes), (2, text.len() + 6));
        assert_eq!(demand.capacities.owners, 1);
        assert_eq!(demand.capacities.theatre_headers, 1);
        assert!(
            !world
                .reads
                .borrow()
                .iter()
                .any(|&(at, len)| at == 0xf400 && len == text.len())
        );
        assert!(world.touches(NAVY, 0x90));
    }

    #[test]
    fn invalid_name_demand_keeps_other_valid_name_and_movement_floors() {
        let mut world = World::new();
        world.pointer(TASK + 0x258 + 24, 1); // invalid inline capacity
        world.bytes[0xdc00 + 0x258..0xdc00 + 0x258 + 6].copy_from_slice(b"Stored");
        world.pointer(0xdc00 + 0x258 + 16, 6);
        let mut floor = 0;
        let demand = observe(&world, 0, STATE, &mut floor);
        assert_eq!(
            demand.refusal(),
            Some("task-force cached name header invalid")
        );
        assert_eq!((demand.name_rows, demand.name_bytes), (2, 6));
        assert_eq!(demand.capacities.owners, 1);
        assert_eq!(demand.capacities.foreign, 3);
        assert_eq!(demand.shared.reserved_provinces, 1);
    }

    #[test]
    fn supply_hint_sizes_stationary_task_forces_from_headers_without_payloads() {
        let mut world = World::new();
        for (task, data, count) in [(TASK, 0x100000, 10_000), (0xdc00, 0x200000, 20_000)] {
            world.word(task + cunit::ORDINARY_ROUTE_COUNT, 0);
            world.array(task + engine::cunit_supply_inputs::ORIGINS, data, count);
        }
        let demand = observe(&world, 0, STATE, &mut 0);
        assert_eq!(demand.refusal(), None);
        assert_eq!(demand.supply_origins, 30_000);
        assert_eq!(demand.name_rows, 2);
        assert_eq!(demand.capacities.owners, 1);
        assert!(!world.touches(
            0x100000,
            10_000 * engine::cunit_supply_inputs::ORIGIN_STRIDE
        ));
        assert!(!world.touches(
            0x200000,
            20_000 * engine::cunit_supply_inputs::ORIGIN_STRIDE
        ));
        assert!(!world.touches(TASK + engine::cunit_supply_inputs::ASKED, 8));
        assert!(world.touches(NAVY, 0x90));
        let mut capacity = crate::extended::naval_counters::Capacities::default();
        let initial = capacity;
        capacity.raise_for_supply_origins(demand.supply_origins);
        assert!(capacity != initial);
        let grown = capacity;
        capacity.raise_for_supply_origins(0);
        assert!(capacity == grown);
    }

    #[test]
    fn port_hint_shares_the_fleet_visitor_and_sizes_large_reference_headers_without_payloads() {
        let mut world = World::new();
        world.bytes.resize(0x11000, 0);
        // The older movement fixture needed only a short subset of each TF.
        // Port+740 would alias the second TF's parent at that old spacing.
        // Give the second complete object its own range before adding fields.
        let second = world.bytes[0xdc00..0xe400].to_vec();
        world.bytes[0xe800..0xf000].copy_from_slice(&second);
        world.pointer(0x3388, 0xe800);
        for (task, refs, count, strings) in [
            (TASK, 0x100000, 10_000, 0x10000),
            (0xe800, 0x200000, 20_000, 0x10100),
        ] {
            world.word(task + cunit::ORDINARY_ROUTE_COUNT, 0);
            world.word(task + ctask_force::MISSION, 2);
            world.pointer(
                task + ctask_force::NAVAL_MISSION,
                engine::cnaval_mission::VTABLE_RVA,
            );
            world.pointer(task + ctask_force::NAVAL_MISSION + 8, task);
            world.word(
                task + ctask_force::NAVAL_MISSION + engine::cnaval_mission::ENGAGEMENT_RULE,
                2,
            );
            world.array(task + 0x610, refs, count);
            world.array(task + 0x740, strings, 2);
            for index in 0..2 {
                let at = strings + index * 32;
                world.bytes[at..at + 3].copy_from_slice(&[b'x', 0, 0xff]);
                world.pointer(at + 16, 3);
                world.pointer(at + 24, 15);
            }
        }
        let countries = ccountry::Reader::new(&world, 0, STATE).unwrap();
        let observed = owners(&world, 0, &countries).unwrap();
        assert_eq!(
            (
                observed.port.searchers,
                observed.port.avoided,
                observed.port.bytes,
                observed.port.references
            ),
            (30_000, 4, 12, 30_002)
        );
        assert!(observed.port.refusal.is_none());
        assert_eq!(observed.name_rows, 2);
        assert_eq!(observed.supply_origins, 0);
        assert!(!world.touches(0x100000, 10_000 * 8));
        assert!(!world.touches(0x200000, 20_000 * 8));
        let mut capacity = crate::extended::naval_counters::Capacities::default();
        capacity.raise_for_port(observed.port).unwrap();
        let grown = capacity;
        capacity
            .raise_for_port(engine::ctask_force::port_mission::Demand::default())
            .unwrap();
        assert!(capacity == grown);
    }

    #[test]
    fn invalid_supply_header_keeps_other_supply_name_and_movement_floors() {
        let mut world = World::new();
        // A positive count with no data is a failure, not an empty origin list.
        world.array(TASK + engine::cunit_supply_inputs::ORIGINS, 0, 3);
        world.array(
            0xdc00 + engine::cunit_supply_inputs::ORIGINS,
            0x100000,
            7000,
        );
        let demand = observe(&world, 0, STATE, &mut 0);
        assert_eq!(demand.refusal(), Some("unit supply origin header invalid"));
        assert_eq!(demand.supply_origins, 7000);
        assert_eq!(demand.name_rows, 2);
        assert_eq!(demand.capacities.owners, 1);
        assert_eq!(demand.capacities.foreign, 3);
        assert_eq!(demand.shared.reserved_provinces, 1);
    }

    #[test]
    fn supply_header_fault_is_named_and_preserves_independent_hints() {
        let mut world = World::new();
        world.fault = Some(TASK + engine::cunit_supply_inputs::ORIGINS);
        world.array(
            0xdc00 + engine::cunit_supply_inputs::ORIGINS,
            0x100000,
            7000,
        );
        let demand = observe(&world, 0, STATE, &mut 0);
        assert_eq!(demand.refusal(), Some("unit supply inputs unreadable"));
        assert_eq!(demand.supply_origins, 7000);
        assert_eq!(demand.name_rows, 2);
        assert_eq!(demand.capacities.owners, 1);
        assert_eq!(demand.capacities.foreign, 3);
    }

    #[test]
    fn failed_domain_retains_header_floors_without_estimated_theatre_outputs() {
        let mut world = World::new();
        world.pointer(0xa200, 0);
        let mut foreign_floor = 0;
        let demand = observe(&world, 0, STATE, &mut foreign_floor);
        assert_eq!(
            demand.refusal(),
            Some("naval movement province identity mismatch")
        );
        assert_eq!(demand.capacities.provinces, 4);
        assert_eq!(demand.capacities.theatre_headers, 1);
        assert_eq!(demand.capacities.theatre_provinces, 0);
        assert_eq!(demand.shared.reserved_provinces, 0);
        assert_eq!(demand.shared.nearest_bases, 12);
    }

    #[test]
    fn native_header_fault_does_not_retry_for_foreign_capacity_or_discard_other_floors() {
        let mut world = World::new();
        world.fault = Some(NAVY + native::NEAREST_BASES);
        let mut foreign_floor = 0;
        let demand = observe(&world, 0, STATE, &mut foreign_floor);
        assert!(demand.refusal().is_some());
        assert_eq!(demand.capacities.foreign, 3);
        assert_eq!(foreign_floor, 3);
        assert_eq!(demand.capacities.hubs, 3);
        assert_eq!(demand.capacities.physical_slots, 4);
        assert_eq!(demand.shared.nearest_bases, 0);
        assert_eq!(
            world
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, len)| at == NAVY + native::NEAREST_BASES && len == 16)
                .count(),
            1
        );
    }
}
