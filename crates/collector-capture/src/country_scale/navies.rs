use crate::extended::PhysicalNavyCapacities;
use engine_native::{Memory, ccountry, cfleet};
use engine_naval_history::country_naval_movement_inputs as native;

#[derive(Clone, Copy, Default)]
pub struct Demand {
    pub capacities: PhysicalNavyCapacities,
    pub name_rows: usize,
    pub name_bytes: usize,
    pub supply_origins: usize,
    pub port: engine_native::ctask_force::port_mission::Demand,
    first_reason: Option<&'static str>,
}

impl Demand {
    pub fn refusal(&self) -> Option<&'static str> {
        self.first_reason
    }

    fn refuse(&mut self, reason: &'static str) {
        self.first_reason.get_or_insert(reason);
    }

    fn bounds(&mut self, bounds: native::Bounds) {
        match bounds.navy_slots {
            Ok(count) => self.capacities.navies = self.capacities.navies.max(count),
            Err(reason) => self.refuse(reason),
        }
    }

    fn names(&mut self, observed: &TaskForces) {
        self.name_rows = self.name_rows.max(observed.name_rows);
        self.name_bytes = self.name_bytes.max(observed.name_bytes);
        if let Some(reason) = observed.name_failure {
            self.refuse(reason);
        }
    }

    fn supply(&mut self, observed: &TaskForces) {
        self.supply_origins = self.supply_origins.max(observed.supply_origins);
        if let Some(reason) = observed.supply_failure {
            self.refuse(reason);
        }
    }

    fn port(&mut self, observed: &TaskForces) {
        self.port.searchers = self.port.searchers.max(observed.port.searchers);
        self.port.avoided = self.port.avoided.max(observed.port.avoided);
        self.port.bytes = self.port.bytes.max(observed.port.bytes);
        self.port.references = self.port.references.max(observed.port.references);
        if engine_native::ctask_force::port_mission::reference_slots(self.port.references).is_none()
        {
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

struct TaskForces {
    failure: Option<&'static str>,
    name_rows: usize,
    name_bytes: usize,
    name_failure: Option<&'static str>,
    supply_origins: usize,
    supply_failure: Option<&'static str>,
    port: engine_native::ctask_force::port_mission::Demand,
}

fn task_forces<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
) -> TaskForces {
    let mut observed = TaskForces {
        failure: None,
        name_rows: 0,
        name_bytes: 0,
        name_failure: None,
        supply_origins: 0,
        supply_failure: None,
        port: engine_native::ctask_force::port_mission::Demand::default(),
    };
    let walk = cfleet::visit_task_forces(memory, base, countries, |event| {
        let cfleet::Visit::TaskForce(visit, _) = event else {
            return Ok(());
        };
        match observed.name_rows.checked_add(1) {
            Some(count) => observed.name_rows = count,
            None => {
                observed
                    .name_failure
                    .get_or_insert("task-force name sizing row overflow");
            }
        }
        match engine_native::ctask_force::cached_name_demand(memory, base, visit.task, visit.id) {
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
        match engine_native::cunit_supply_inputs::origin_demand(memory, visit.task) {
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
        let port =
            engine_native::ctask_force::port_mission::demand(memory, base, visit.task, visit.kind);
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
    observed
}

pub fn observe(memory: &impl Memory, base: usize, state: usize) -> Demand {
    let mut demand = Demand::default();
    let reader = native::Reader::new(memory, base, state);
    let bounds = reader.bounds();
    demand.bounds(bounds);
    match ccountry::Reader::new(memory, base, state) {
        Ok(countries) => {
            let observed = task_forces(memory, base, &countries);
            demand.names(&observed);
            demand.supply(&observed);
            if let Some(reason) = observed.failure {
                demand.refuse(reason);
            }
            demand.port(&observed);
        }
        Err(reason) => demand.refuse(reason),
    }
    demand
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{cgame_state, cprovince, ctask_force, ctheatre, cunit};
    use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};
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
                world.pointer(country, ccountry::VTABLE_RVA.0);
                world.word(country + ccountry::ID, id);
                world.word(0x3100 + slot * 4, link);
            }
            world.array(COUNTRY + ccountry::FLEETS, 0x3300, 1);
            world.pointers(0x3300, &[0xd000]);
            world.pointer(0xd000, cfleet::VTABLE_RVA.0);
            world.pointer(
                0xd000 + cfleet::HANDLE,
                (1 << 32) | cfleet::HANDLE_TYPE as usize,
            );
            world.pointer(0xd000 + cfleet::OWNER, COUNTRY);
            world.array(0xd000 + cfleet::TASK_FORCES, 0x3380, 2);
            world.pointers(0x3380, &[TASK, 0xdc00]);
            for (id, task) in [(1, TASK), (2, 0xdc00)] {
                world.pointer(task, ctask_force::VTABLE_RVA.0);
                world.pointer(
                    task + ctask_force::SECONDARY_BASE,
                    ctask_force::SECONDARY_VTABLE_RVA.0,
                );
                world.pointer(
                    task + cunit::HANDLE,
                    (id << 32) | ctask_force::DEFAULT_HANDLE_TYPE as usize,
                );
                world.pointer(task + ctask_force::FLEET, 0xd000);
                world.pointer(task + 0x258 + 24, 15);
                world.word(task + cunit::ROSTER_OWNER, 3);
                world.array(task + cunit::ORDINARY_ROUTE_DATA, 0xf000, 1);
            }
            world.pointer(STATE + cgame_state::STRATEGIC_NAVY_MANAGER, 0x6000);
            world.pointer(0x6000, cstrategic_navy_manager::VTABLE_RVA.0);
            world.array(0x6000 + cstrategic_navy_manager::NAVIES, 0x6100, 4);
            world.pointers(0x6100, &[0, NAVY, 0x7200, 0x7400]);
            for navy in [NAVY, 0x7200, 0x7400] {
                world.pointer(navy, cstrategic_navy::VTABLE_RVA.0);
                world.pointer(
                    navy + cstrategic_navy::SECONDARY,
                    cstrategic_navy::SECONDARY_VTABLE_RVA.0,
                );
                world.pointer(navy + native::MANAGER, 0x6000);
            }
            world.array(STATE + cgame_state::PROVINCES, 0x3200, 5);
            world.pointers(0x3200, &[0, 0xa000, 0xa200, 0xa400, 0xa600]);
            for (id, province, area) in [
                (1, 0xa000, 0xe100),
                (2, 0xa200, 0xe200),
                (3, 0xa400, 0),
                (4, 0xa600, 0xe300),
            ] {
                world.pointer(province, cprovince::VTABLE_RVA.0);
                world.word(province + cprovince::ID, id);
                world.pointer(province + cprovince::CONTROLLER_AREA, area);
                if area != 0 {
                    world.pointer(area, engine_native::ccontroller_area::VTABLE_RVA.0);
                }
            }
            for (country, roster, theatre, owner, id, areas, values) in [
                (COUNTRY, 0xb000, 0xb100, 3, 1, 0xb600, &[0xe100, 0][..]),
                (0x4000, 0xb800, 0xb900, 0, 2, 0xbc00, &[0xe200][..]),
            ] {
                world.array(country + ccountry::THEATRES, roster, 1);
                world.pointers(roster, &[theatre]);
                world.pointer(theatre, ctheatre::VTABLE_RVA.0);
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
    fn attach_hint_sizes_slots_without_navy_theatre_owner_or_area_reads() {
        let world = World::new();
        let demand = observe(&world, 0, STATE);
        assert_eq!(demand.refusal(), None);
        assert_eq!(demand.capacities.navies, 4);
        assert!(!world.touches(0xa200 + cprovince::CONTROLLER_AREA, 8));
        assert_eq!(demand.name_rows, 2);
        for navy in [NAVY, 0x7200, 0x7400] {
            assert!(!world.touches(navy, 0x90));
        }
        assert!(!world.touches(0xb000, 8));
        assert!(!world.touches(0xb800, 8));
        for task in [TASK, 0xdc00] {
            assert!(!world.touches(task + cunit::ROSTER_OWNER, 4));
            assert!(!world.touches(task + cunit::ORDINARY_ROUTE_DATA, 16));
        }
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
        let demand = observe(&world, 0, STATE);
        assert_eq!(demand.refusal(), None);
        assert_eq!((demand.name_rows, demand.name_bytes), (2, text.len() + 6));
        assert!(
            !world
                .reads
                .borrow()
                .iter()
                .any(|&(at, len)| at == 0xf400 && len == text.len())
        );
        assert!(!world.touches(NAVY, 0x90));
    }

    #[test]
    fn invalid_name_demand_keeps_other_valid_name_floors() {
        let mut world = World::new();
        world.pointer(TASK + 0x258 + 24, 1);
        world.bytes[0xdc00 + 0x258..0xdc00 + 0x258 + 6].copy_from_slice(b"Stored");
        world.pointer(0xdc00 + 0x258 + 16, 6);
        let demand = observe(&world, 0, STATE);
        assert_eq!(
            demand.refusal(),
            Some("task-force cached name header invalid")
        );
        assert_eq!((demand.name_rows, demand.name_bytes), (2, 6));
    }

    #[test]
    fn supply_hint_sizes_stationary_task_forces_from_headers_without_payloads() {
        let mut world = World::new();
        for (task, data, count) in [(TASK, 0x100000, 10_000), (0xdc00, 0x200000, 20_000)] {
            world.word(task + cunit::ORDINARY_ROUTE_COUNT, 0);
            world.array(
                task + engine_native::cunit_supply_inputs::ORIGINS,
                data,
                count,
            );
        }
        let demand = observe(&world, 0, STATE);
        assert_eq!(demand.refusal(), None);
        assert_eq!(demand.supply_origins, 30_000);
        assert_eq!(demand.name_rows, 2);
        assert!(!world.touches(
            0x100000,
            10_000 * engine_native::cunit_supply_inputs::ORIGIN_STRIDE
        ));
        assert!(!world.touches(
            0x200000,
            20_000 * engine_native::cunit_supply_inputs::ORIGIN_STRIDE
        ));
        assert!(!world.touches(TASK + engine_native::cunit_supply_inputs::ASKED, 8));
        assert!(!world.touches(NAVY, 0x90));
        let mut capacity = collector_naval_counters::naval_counters::Capacities::default();
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
                engine_native::cnaval_mission::VTABLE_RVA.0,
            );
            world.pointer(task + ctask_force::NAVAL_MISSION + 8, task);
            world.word(
                task + ctask_force::NAVAL_MISSION + engine_native::cnaval_mission::ENGAGEMENT_RULE,
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
        let observed = task_forces(&world, 0, &countries);
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
        let mut capacity = collector_naval_counters::naval_counters::Capacities::default();
        capacity.raise_for_port(observed.port).unwrap();
        let grown = capacity;
        capacity
            .raise_for_port(engine_native::ctask_force::port_mission::Demand::default())
            .unwrap();
        assert!(capacity == grown);
    }

    #[test]
    fn invalid_supply_header_keeps_other_supply_and_name_floors() {
        let mut world = World::new();
        world.array(TASK + engine_native::cunit_supply_inputs::ORIGINS, 0, 3);
        world.array(
            0xdc00 + engine_native::cunit_supply_inputs::ORIGINS,
            0x100000,
            7000,
        );
        let demand = observe(&world, 0, STATE);
        assert_eq!(demand.refusal(), Some("unit supply origin header invalid"));
        assert_eq!(demand.supply_origins, 7000);
        assert_eq!(demand.name_rows, 2);
    }

    #[test]
    fn supply_header_fault_is_named_and_preserves_independent_hints() {
        let mut world = World::new();
        world.fault = Some(TASK + engine_native::cunit_supply_inputs::ORIGINS);
        world.array(
            0xdc00 + engine_native::cunit_supply_inputs::ORIGINS,
            0x100000,
            7000,
        );
        let demand = observe(&world, 0, STATE);
        assert_eq!(demand.refusal(), Some("unit supply inputs unreadable"));
        assert_eq!(demand.supply_origins, 7000);
        assert_eq!(demand.name_rows, 2);
    }
}
