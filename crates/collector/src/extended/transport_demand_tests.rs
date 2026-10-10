use super::*;
use engine::{cgame_state, cstrategic_air, cstrategic_air_manager, cstrategic_navy_manager};

struct World {
    bytes: Vec<u8>,
    countries: usize,
    airs: usize,
}
impl Memory for World {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        if at == cgame_state::SINGLETON_RVA && output.len() == 8 {
            output.copy_from_slice(&0x1000_u64.to_le_bytes());
            return true;
        }
        let Some(source) = at
            .checked_add(output.len())
            .and_then(|end| self.bytes.get(at..end))
        else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}
impl World {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn header(&mut self, at: usize, pointer: usize, count: usize) {
        self.word(at, pointer as u64);
        self.put(at + 8, &(count as u32).to_le_bytes());
        self.put(at + 12, &(count as u32).to_le_bytes());
    }
    fn new(count: usize) -> Self {
        let countries = 0x30000;
        let airs = countries + (count + 1) * 0x1500;
        let mut world = Self {
            bytes: vec![0; airs + (count + 1) * 0x200],
            countries,
            airs,
        };
        world.header(0x1000 + cgame_state::COUNTRIES, 0x4000, count + 1);
        world.header(0x1000 + cgame_state::COUNTRY_LINKS, 0xe000, count + 1);
        world.word(0x1000 + cgame_state::STRATEGIC_AIR_MANAGER, 0x2000);
        world.word(0x2000, cstrategic_air_manager::VTABLE_RVA as u64);
        world.header(
            0x2000 + cstrategic_air_manager::COUNTRY_AIR,
            0x14000,
            count + 1,
        );
        world.word(0x1000 + cgame_state::STRATEGIC_NAVY_MANAGER, 0x2100);
        world.word(0x1000 + cgame_state::WEATHER_MANAGER, 0x2180);
        world.word(0x2180, engine::cweather_manager::VTABLE_RVA as u64);
        world.word(0x2100, cstrategic_navy_manager::VTABLE_RVA as u64);
        // A physical holder domain larger than the country domain.
        world.header(0x2100 + cstrategic_navy_manager::NAVIES, 0x1e000, count + 7);
        world.word(0x29400, engine::ccountry_intel::VTABLE_RVA as u64);
        for index in 0..5 {
            world.word(0x29100 + index * 8, (0x29500 + index * 8) as u64);
        }
        for slot in 0..=count {
            let country = countries + slot * 0x1500;
            world.word(0x4000 + slot * 8, country as u64);
            world.word(country, ccountry::VTABLE_RVA as u64);
            world.put(country + ccountry::ID, &(slot as u32).to_le_bytes());
            world.put(0xe000 + slot * 4, &(slot as u32).to_le_bytes());
            let radar = country + ccountry::RADARS_POOL;
            world.word(radar, engine::cradars_pool::VTABLE_RVA as u64);
            world.word(radar + 8, engine::cradars_pool::SECONDARY_VTABLE_RVA as u64);
            world.put(
                radar + engine::cradars_pool::COUNTRY,
                &(slot as u32).to_le_bytes(),
            );
            world.word(country + ccountry::INTEL_TABLE, 0x29400);
            let convoys = country + ccountry::CONVOY_STATE;
            world.word(convoys, engine::cconvoys::VTABLE_RVA as u64);
            world.word(
                convoys + engine::cconvoys::EQUIPMENT_POOL,
                engine::cequipment_variant_pool::VTABLE_RVA as u64,
            );
            world.header(
                convoys + engine::cconvoys::EQUIPMENT_POOL + engine::cequipment_variant_pool::ITEMS,
                0x29000,
                2,
            );
            world.header(convoys + engine::cconvoys::RESERVATIONS, 0x29100, 3);
            if slot == 0 {
                continue;
            }
            let air = airs + slot * 0x200;
            world.word(0x14000 + slot * 8, air as u64);
            world.word(air, cstrategic_air::VTABLE_RVA as u64);
            world.put(air + cstrategic_air::COUNTRY, &(slot as u32).to_le_bytes());
            world.header(air + cstrategic_air::REGIONAL_MISSIONS, 0x29200, 2);
            world.header(air + 0xe0, 0x29300, 1);
        }
        world
    }
}

#[test]
fn air_sizing_uses_native_24_byte_headers_and_keeps_independent_failed_hint_demand() {
    const REGIONS: usize = 257;
    let mut world = World::new(5000);
    let headers = world.bytes.len();
    world
        .bytes
        .resize(headers + REGIONS * engine::cpdx_array::STRIDE, 0);
    world.header(
        world.airs + 0x200 + cstrategic_air::REGIONAL_MISSIONS,
        headers,
        REGIONS,
    );
    let mut missions = 0;
    for region in 0..REGIONS {
        let count = region % 7 + 1;
        let at = headers + region * engine::cpdx_array::STRIDE;
        world.header(at, 0x7000_0000, count);
        world.word(at + 16, u64::MAX);
        missions += count;
    }
    struct ExactHeaders<'a> {
        world: &'a World,
        headers: usize,
        fault: std::cell::Cell<Option<usize>>,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for ExactHeaders<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((at, output.len()));
            if at < self.headers + REGIONS * engine::cpdx_array::STRIDE
                && at + output.len() > self.headers
            {
                assert!(at >= self.headers);
                let offset = (at - self.headers) % engine::cpdx_array::STRIDE;
                assert!(
                    offset + output.len() <= 16,
                    "sizing must not read regional allocator/padding bytes"
                );
            }
            if self
                .fault
                .get()
                .is_some_and(|fault| at <= fault && fault < at + output.len())
            {
                return false;
            }
            self.world.copy(at, output)
        }
    }
    let memory = ExactHeaders {
        world: &world,
        headers,
        fault: std::cell::Cell::new(None),
        reads: Default::default(),
    };
    let demand = observe(&memory, 0).unwrap().unwrap();
    assert_eq!(demand.refusal, None);
    assert_eq!(demand.air[..4], [5001, 4999 * 2 + REGIONS, missions, 5000]);
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == 0x7000_0000)
    );
    memory.fault.set(Some(headers + engine::cpdx_array::STRIDE));
    memory.reads.borrow_mut().clear();
    let demand = observe(&memory, 0).unwrap().unwrap();
    assert_eq!(
        demand.refusal,
        Some("transport air sizing regional headers unreadable")
    );
    assert_eq!(demand.air[..4], [5001, 4999 * 2 + REGIONS, 1, 5000]);
    assert_eq!(demand.convoys.pool, 10000);
    assert_eq!(demand.missions.navies, 5007);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == headers + engine::cpdx_array::STRIDE)
            .count(),
        1
    );
}

#[test]
fn startup_and_same_population_reload_size_every_transport_family_from_live_headers() {
    let mut world = World::new(5000);
    let mut capacity = super::super::Capacities::default();
    let _ = capacity.presize_live_inputs(&world, 0);
    let demand = observe(&world, 0).unwrap().unwrap();
    assert_eq!(demand.refusal, None);
    assert_eq!(demand.missions.navies, 5007);
    assert_eq!(demand.convoys.pool, 10000);
    assert_eq!(demand.convoys.reservations, 15000);
    assert_eq!(demand.air, [5001, 10000, 0, 5000, 0]);
    // New arrays at unchanged country/holder counts must still be observed.
    world.header(
        world.airs + 0x200 + cstrategic_air::REGIONAL_MISSIONS,
        0x29200,
        4,
    );
    world.header(
        world.countries + 0x1500 + ccountry::CONVOY_STATE + engine::cconvoys::RESERVATIONS,
        0x29100,
        5,
    );
    let _ = capacity.presize_live_inputs(&world, 0);
    let demand = observe(&world, 0).unwrap().unwrap();
    assert_eq!(demand.air[1], 10002);
    assert_eq!(demand.convoys.reservations, 15002);
    // The prepared pack must admit every physical navy on its first pass,
    // including slots beyond the country domain.
    let mut appearance =
        super::super::country_appearance::Captured::with_capacities(&capacity.country_appearance);
    appearance.convoys.begin(7, 5001);
    let countries = ccountry::Reader::new(&world, 0, 0x1000).unwrap();
    appearance.spotting_owners.begin(7, Ok(5001));
    appearance
        .spotting_owners
        .country(&world, 0, &countries, 0, Ok(None));
    for slot in 1..=5000 {
        appearance.convoys.observe(
            &world,
            0,
            countries.at(slot).unwrap().unwrap(),
            Some(Ok(engine::cconvoys::transport::Cache {
                valid: 1,
                raw: Some(0),
            })),
        );
        appearance
            .spotting_owners
            .country(&world, 0, &countries, slot, countries.at(slot));
    }
    appearance
        .convoys
        .finish(engine::walk::Outcome::Rows { len: 5000 });
    let convoy = appearance.convoys.projection(5000).unwrap().unwrap();
    assert_eq!(convoy.pool.unwrap().len(), 2);
    assert_eq!(convoy.reservations.unwrap().len(), 3);
    appearance.spotting_owners.begin_regions(Ok(0));
    appearance.spotting_owners.finish();
    let spotting = appearance.spotting_owners.projection().unwrap().unwrap();
    assert!(spotting.owners_complete);
    assert_eq!(spotting.owners.len(), 5000);
    let transport = appearance.transport_missions_mut();
    transport.begin(7, Ok(5007));
    for slot in 0..5007 {
        transport.navy(slot, false, Ok(0));
        transport.access_modes(&world, Ok(Array::default()));
    }
    transport.finish();
    assert_eq!(
        transport
            .projection()
            .unwrap()
            .unwrap()
            .navies
            .unwrap()
            .len(),
        5007
    );
}

#[test]
fn live_nonempty_ledgers_and_access_prepare_the_first_physical_navy_pass() {
    let mut world = World::new(5000);
    let navies = world.bytes.len();
    world.bytes.resize(navies + 5007 * 0x200, 0);
    world.header(0x29800, 0x29840, 2);
    world.header(0x29818, 0x29840, 3);
    world.put(0x29880, &[0, 1, 2]);
    for slot in 1..5007 {
        let navy = navies + slot * 0x200;
        world.word(0x1e000 + slot * 8, navy as u64);
        world.word(navy, engine::cstrategic_navy::VTABLE_RVA as u64);
        world.word(
            navy + 8,
            engine::cstrategic_navy::SECONDARY_VTABLE_RVA as u64,
        );
        world.word(
            navy + engine::country_naval_movement_inputs::MANAGER,
            0x2100,
        );
        world.header(navy + engine::cstrategic_navy::REGION_MISSIONS, 0x29800, 2);
        world.header(navy + engine::cstrategic_navy::ACCESS_MODES, 0x29880, 3);
    }
    let demand = observe(&world, 0).unwrap().unwrap();
    assert_eq!(demand.refusal, None);
    assert_eq!(demand.missions.ledgers, 10012);
    assert_eq!(demand.missions.occurrences, 25030);
    assert_eq!(demand.missions.receivers, 25030);
    assert_eq!(demand.missions.access_modes, 15018);
    let mut capacity = super::super::Capacities::default();
    let _ = capacity.presize_live_inputs(&world, 0);
    let mut appearance =
        super::super::country_appearance::Captured::with_capacities(&capacity.country_appearance);
    let transport = appearance.transport_missions_mut();
    transport.begin(7, Ok(5007));
    transport.navy(0, false, Ok(0));
    transport.access_modes(&world, Ok(Array::default()));
    for slot in 1..5007 {
        transport.navy(slot, true, Ok(2));
        transport.access_modes(
            &world,
            Ok(Array {
                data: 0x29880,
                count: 3,
            }),
        );
        transport.ledger(0, 2, Ok(&[0, 0]));
        transport.ledger(1, 3, Ok(&[0, 0, 0]));
    }
    transport.finish();
    let navies = transport.projection().unwrap().unwrap().navies.unwrap();
    assert_eq!(navies.len(), 5007);
    assert_eq!(navies[0].access_modes, Some(vec![]));
    assert_eq!(navies[0].ledgers, Some(vec![]));
    for navy in navies.into_iter().skip(1) {
        assert_eq!(navy.access_modes, Some(vec![0, 1, 2]));
        let ledgers = navy.ledgers.unwrap();
        assert_eq!(ledgers.len(), 2);
        assert_eq!(ledgers[0].occurrences, Some(vec![0, 0]));
        assert_eq!(ledgers[1].occurrences, Some(vec![0, 0, 0]));
    }
}

#[test]
fn failed_family_is_named_counted_and_cannot_withhold_independent_live_demand() {
    let mut world = World::new(1500);
    world.word(world.countries + 0x1500 + ccountry::CONVOY_STATE, 0);
    let before = serde_json::to_value(status()).unwrap()["convoys"]["failed_observations"]
        .as_u64()
        .unwrap();
    let demand = observe(&world, 0).unwrap().unwrap();
    assert_eq!(
        demand.refusal,
        Some("transport embedded CConvoys identity mismatch")
    );
    let status = serde_json::to_value(status()).unwrap();
    assert!(status["convoys"]["failed_observations"].as_u64().unwrap() > before);
    assert_eq!(
        status["convoys"]["last_reason"],
        "transport embedded CConvoys identity mismatch"
    );
    assert_eq!(demand.convoys.pool, 0);
    assert_eq!(demand.missions.navies, 1507);
    assert_eq!(demand.air[1], 3000);
}

#[test]
fn original_spotting_headers_capture_five_thousand_owners_and_same_population_array_growth() {
    use super::super::transport_spotting_context::{Capacities, Captured};
    let mut world = World::new(5000);
    for (index, value) in [-7_i32, 23, 91, -101, 211].into_iter().enumerate() {
        world.put(0x29a00 + index * 4, &value.to_le_bytes());
    }
    for (index, value) in [-51_i64, 83].into_iter().enumerate() {
        world.put(
            0x29b00 + index * engine::country_intel_row::STRIDE + engine::country_intel_row::NAVY,
            &value.to_le_bytes(),
        );
    }
    world.header(0x29400 + engine::ccountry_intel::DATA, 0x29b00, 2);
    for slot in 1..=5000 {
        world.header(
            world.countries
                + slot * 0x1500
                + ccountry::RADARS_POOL
                + engine::transport_spotting_context::RADAR_COUNTS,
            0x29a00,
            3,
        );
    }
    let mut captured = Captured::new(Capacities::default());
    for hour in 7..=8 {
        if hour == 8 {
            world.header(
                world.countries
                    + 0x1500
                    + ccountry::RADARS_POOL
                    + engine::transport_spotting_context::RADAR_COUNTS,
                0x29a00,
                5,
            );
        }
        let countries = ccountry::Reader::new(&world, 0, 0x1000).unwrap();
        captured.begin(hour, Ok(countries.count() as usize));
        captured.begin_regions(Ok(0));
        captured.country(&world, 0, &countries, 0, Ok(None));
        for slot in 1..=5000 {
            captured.country(&world, 0, &countries, slot, countries.at(slot));
        }
        captured.finish();
        let packet = captured.projection().unwrap().unwrap();
        assert!(packet.owners_complete && packet.regions_complete);
        assert_eq!(packet.owners.len(), 5000);
        for (index, owner) in packet.owners.iter().enumerate() {
            assert_eq!(owner.owner_slot, index as u32 + 1);
            let expected = if hour == 8 && index == 0 {
                vec![-7, 23, 91, -101, 211]
            } else {
                vec![-7, 23, 91]
            };
            assert_eq!(owner.radar.as_ref().unwrap().counts, Some(expected));
            assert_eq!(
                owner.navy_intel.as_ref().unwrap().lanes,
                Some(vec![-51, 83])
            );
        }
    }
}
