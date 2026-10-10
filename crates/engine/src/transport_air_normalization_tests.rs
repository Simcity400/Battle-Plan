use super::*;
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Default)]
struct Sparse {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<BTreeMap<usize, usize>>,
}
impl Sparse {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, byte)| (at + index, *byte)),
        );
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.put(at, &(data as u64).to_le_bytes());
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn row(&mut self, at: usize, key: u32, raw: i64) {
        self.put(at, &key.to_le_bytes());
        self.put(at + 4, &0_u32.to_le_bytes());
        self.put(at + 8, &raw.to_le_bytes());
    }
    fn metadata(&mut self, key: u32) {
        let table = 0x100_000usize;
        self.put(
            modifier_definition::TABLE_RVA,
            &(table as u64).to_le_bytes(),
        );
        self.put(
            table
                + key as usize * modifier_definition::STRIDE
                + modifier_definition::FEATURE_SELECTOR,
            &0_u32.to_le_bytes(),
        );
        self.put(
            table + key as usize * modifier_definition::STRIDE + modifier_definition::FLAGS,
            &0_u32.to_le_bytes(),
        );
    }
}
impl Memory for Sparse {
    fn copy(&self, at: usize, destination: &mut [u8]) -> bool {
        *self.reads.borrow_mut().entry(at).or_default() += 1;
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(at + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

#[test]
fn live_weather_entry_hints_count_all_headers_without_acquiring_their_values() {
    let mut memory = Sparse::default();
    memory.put(
        0x1000 + crate::cgame_state::WEATHER_MANAGER,
        &0x2000_u64.to_le_bytes(),
    );
    memory.put(
        0x2000,
        &(crate::cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
    );
    memory.array(0x2000 + crate::cweather_manager::REGIONS, 0x3000, 5000);
    memory.array(
        0x2000 + crate::cweather_manager::PROVINCE_CLOCKS,
        0x20_0000,
        5000,
    );
    for slot in 0..5000 {
        memory.array(
            0x3000
                + slot * crate::sweather_per_region::STRIDE
                + crate::sweather_per_region::MODIFIERS,
            0x80_0000,
            3,
        );
        memory.array(0x20_0000 + slot * 0x330 + 0x280, 0x80_0000, 7);
    }
    assert_eq!(weather_entry_demand(&memory, 0, 0x1000), Ok(50000));
    assert!(!memory.reads.borrow().contains_key(&0x80_0000));
}

#[test]
fn live_wing_entry_hints_follow_country_override_and_only_living_ace_graphs() {
    use crate::{
        cace, cair_wing, cair_wing_pool, cgame_state, creference_registry as registry,
        cstrategic_air, cstrategic_air_manager,
    };
    struct World {
        fields: Sparse,
        registry: registry::tests::Fixture,
    }
    impl Memory for World {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.fields.copy(at, output) || self.registry.copy(at, output)
        }
    }
    let game = 0x10000;
    let countries = 0x14000;
    let links = 0x14040;
    let manager = 0x1b000;
    let roots = 0x1c000;
    let air = 0x1d000;
    let pools = 0x1e000;
    let pool = 0x1f000;
    let wings = 0x20000;
    let wing = 0x21000;
    let ace = 0x23000;
    let kind = 0x24000;
    let reference = 9_u64 << 32 | u64::from(registry::DYNAMIC_TYPE_BASE);
    let mut memory = World {
        fields: Sparse::default(),
        registry: registry::tests::Fixture::new(reference as u32),
    };
    memory.registry.entry(
        registry::tests::Fixture::index(reference),
        1,
        reference,
        ace as u64,
    );
    let fields = &mut memory.fields;
    fields.array(game + cgame_state::COUNTRIES, countries, 4);
    fields.array(game + cgame_state::COUNTRY_LINKS, links, 4);
    fields.put(countries, &[0; 32]);
    fields.put(links, &[0; 16]);
    for (slot, country, entries) in [(1, 0x15000, 17), (2, 0x17000, 5), (3, 0x19000, 13)] {
        fields.word(countries + slot * 8, country as u64);
        fields.put(links + slot * 4, &(slot as u32).to_le_bytes());
        fields.word(country, ccountry::VTABLE_RVA as u64);
        fields.put(country + ccountry::ID, &(slot as u32).to_le_bytes());
        fields.array(country + ccountry::MODIFIER_ACCUMULATOR, 0x80_0000, entries);
    }
    fields.word(game + cgame_state::STRATEGIC_AIR_MANAGER, manager as u64);
    fields.word(manager, cstrategic_air_manager::VTABLE_RVA as u64);
    fields.array(manager + cstrategic_air_manager::COUNTRY_AIR, roots, 4);
    fields.put(roots, &[0; 32]);
    fields.word(roots + 8, air as u64);
    fields.word(air, cstrategic_air::VTABLE_RVA as u64);
    fields.put(air + cstrategic_air::COUNTRY, &1_u32.to_le_bytes());
    fields.array(air + cstrategic_air::POOLS, pools, 1);
    fields.word(pools, pool as u64);
    fields.put(pool, &[0; cair_wing_pool::SIZE]);
    fields.word(pool, cair_wing_pool::VTABLE_RVA as u64);
    fields.word(pool + cair_wing_pool::OWNER, air as u64);
    fields.array(pool + cair_wing_pool::WINGS, wings, 1);
    fields.word(wings, wing as u64);
    fields.put(wing, &[0; 0xa00]);
    fields.word(wing, cair_wing::VTABLE_RVA as u64);
    fields.word(
        wing + cair_wing::SECONDARY,
        cair_wing::SECONDARY_VTABLE_RVA as u64,
    );
    fields.word(
        wing + cair_wing::REFERENCE_TYPE,
        1_u64 << 32 | u64::from(cair_wing::TYPE),
    );
    fields.word(wing + cair_wing::POOL, pool as u64);
    fields.word(wing + cair_wing::ACE_REFERENCE, reference);
    fields.word(ace, cace::VTABLE_RVA as u64);
    fields.put(ace + cace::ALIVE, &[1]);
    fields.word(ace + cace::TYPE_DEFINITION, kind as u64);
    fields.word(ace + cace::OWNER_COUNTRY, 0x19000);
    // F48230/F69BC0 read wing+238; GetTypeModifiers@14060D010 returns
    // type+38, then its accumulator+10. No modifier bodies are installed.
    fields.array(wing + 0x238, 0x80_0000, 3);
    fields.array(ace + cace::MODIFIERS, 0x80_0000, 7);
    fields.array(kind + 0x48, 0x80_0000, 11);
    for (override_country, expected) in [(2_i32, 39), (0, 51), (-4, 51)] {
        memory.fields.put(
            wing + cair_wing::COUNTRY_OVERRIDE,
            &override_country.to_le_bytes(),
        );
        assert_eq!(wing_entry_demand(&memory, 0, game), Ok(expected));
        assert!(!memory.fields.reads.borrow().contains_key(&0x80_0000));
    }
    memory
        .fields
        .put(wing + cair_wing::COUNTRY_OVERRIDE, &2_i32.to_le_bytes());
    memory.fields.put(ace + cace::ALIVE, &[0]);
    for at in [
        ace + cace::MODIFIERS,
        ace + cace::TYPE_DEFINITION,
        ace + cace::OWNER_COUNTRY,
    ] {
        memory.fields.bytes.remove(&at);
    }
    memory.fields.reads.borrow_mut().clear();
    assert_eq!(wing_entry_demand(&memory, 0, game), Ok(8));
    for at in [
        ace + cace::MODIFIERS,
        ace + cace::TYPE_DEFINITION,
        ace + cace::OWNER_COUNTRY,
    ] {
        assert!(!memory.fields.reads.borrow().contains_key(&at));
    }
    memory.fields.put(ace + cace::ALIVE, &[1]);
    assert!(wing_entry_demand(&memory, 0, game).is_err());
}

#[test]
fn repeated_direct_scan_and_first_lookup_borrow_one_original_attempt() {
    let mut memory = Sparse::default();
    memory.array(0x1000, 0x2000, 3);
    memory.row(0x2000, 0x6f, 1);
    memory.row(0x2010, 0x6f, 2);
    memory.row(0x2020, 0x70, -4);
    memory.metadata(0x6f);
    memory.metadata(0x70);
    let mut captured = Capture::try_new(1, 3).unwrap();
    let first = captured.modifiers(&memory, 0, 0x1000).unwrap();
    assert_eq!(first.direct, [Some(3), Some(-4)]);
    assert_eq!(first.effective[5], Some(1));
    assert_eq!(first.positive, [Some(2), Some(0)]);
    assert_eq!(
        captured.combat(first.combat.unwrap()).unwrap(),
        [
            CombatModifier {
                lane: 0,
                raw: Some(1)
            },
            CombatModifier {
                lane: 0,
                raw: Some(2)
            },
            CombatModifier {
                lane: 1,
                raw: Some(-4)
            },
        ]
    );
    memory.put(0x2008, &999_i64.to_le_bytes());
    assert_eq!(captured.modifiers(&memory, 0, 0x1000), Ok(first));
    assert_eq!(memory.reads.borrow()[&0x1000], 1);
    assert_eq!(memory.reads.borrow()[&0x2008], 1);
    captured.retire();
    assert_eq!(captured.combat(first.combat.unwrap()).unwrap().len(), 3);
}

#[test]
fn worker_normalization_grows_original_entry_and_identity_journals_without_retry() {
    const ENTRIES: usize = 20_001;
    let mut memory = Sparse::default();
    memory.array(0x1000, 0x800_0000, ENTRIES as u32);
    for entry in 0..ENTRIES {
        memory.row(0x800_0000 + entry * 16, 0x6f, 1);
    }
    memory.metadata(0x6f);
    memory.array(0x7000 + crate::cgame_state::COUNTRIES, 0x8000, 2);
    memory.array(0x7000 + crate::cgame_state::COUNTRY_LINKS, 0x9000, 1);
    for ordinal in 1..=5000 {
        memory.array(0x20_0000 + ordinal * 0x20, 0, 0);
    }
    let countries = ccountry::Reader::new(&memory, 0, 0x7000).unwrap();
    let mut captured = Capture::try_new(0, 0).unwrap();
    captured.enable_worker_growth();
    let first = captured.modifiers(&memory, 0, 0x1000).unwrap();
    assert_eq!(first.direct, [Some(ENTRIES as i64), Some(0)]);
    assert_eq!(
        captured.combat(first.combat.unwrap()).unwrap().len(),
        ENTRIES
    );
    let failed = captured.modifiers(&memory, 0, 0x6000);
    assert_eq!(failed, Err("transport air modifier header refused"));
    let reference = 1_u64 << 32 | u64::from(crate::creference_registry::DYNAMIC_TYPE_BASE);
    let assigned = captured.assigned_aircraft(&memory, 0, &[], reference);
    let ace = captured.ace(&memory, 0, &countries, &[], reference);
    for ordinal in 1..=5000 {
        assert!(
            captured
                .modifiers(&memory, 0, 0x20_0000 + ordinal * 0x20)
                .is_ok()
        );
        let reference =
            ((ordinal + 1) as u64) << 32 | u64::from(crate::creference_registry::DYNAMIC_TYPE_BASE);
        let _ = captured.assigned_aircraft(&memory, 0, &[], reference);
        let _ = captured.ace(&memory, 0, &countries, &[], reference);
    }
    let reads = memory.reads.borrow().clone();
    assert_eq!(captured.modifiers(&memory, 0, 0x1000), Ok(first));
    assert_eq!(captured.modifiers(&memory, 0, 0x6000), failed);
    assert_eq!(
        captured.assigned_aircraft(&memory, 0, &[], reference),
        assigned
    );
    assert_eq!(captured.ace(&memory, 0, &countries, &[], reference), ace);
    assert_eq!(*memory.reads.borrow(), reads);
    assert_eq!(
        captured.combat(first.combat.unwrap()).unwrap().len(),
        ENTRIES
    );
    assert_eq!(memory.reads.borrow()[&0x1000], 1);
    assert_eq!(memory.reads.borrow()[&0x6000], 1);
    captured.retire();
    assert_eq!(
        captured.combat(first.combat.unwrap()).unwrap().len(),
        ENTRIES
    );
}

#[test]
fn failed_direct_leaf_does_not_suppress_an_unrelated_native_lookup() {
    let mut memory = Sparse::default();
    memory.array(0x1000, 0x2000, 3);
    memory.row(0x2000, 2, 25);
    memory.row(0x2010, 0x6f, 1);
    memory.row(0x2020, 0x70, 2);
    memory.metadata(2);
    memory.metadata(0x6f);
    memory.metadata(0x70);
    memory.bytes.remove(&0x2018);
    let mut captured = Capture::try_new(1, 3).unwrap();
    let first = captured.modifiers(&memory, 0, 0x1000).unwrap();
    assert_eq!(first.direct, [None, Some(2)]);
    assert_eq!(captured.combat(first.combat.unwrap()).unwrap()[0].raw, None);
    assert_eq!(first.effective[0], Some(25));
    assert_eq!(first.effective[5], None);
    memory.put(0x2018, &1_i64.to_le_bytes());
    assert_eq!(captured.modifiers(&memory, 0, 0x1000), Ok(first));
    assert_eq!(memory.reads.borrow()[&0x2018], 1);
}

#[test]
fn sampled_and_weather_failures_are_independent_and_never_retried() {
    let mut memory = Sparse::default();
    memory.array(0x1058, 0x2000, 2);
    memory.row(0x2000, 0x6f, -3);
    memory.row(0x2010, 0x70, -5);
    memory.metadata(0x6f);
    memory.metadata(0x70);
    let mut captured = Capture::try_new(2, 2).unwrap();
    let value = captured.environment(
        &memory,
        0,
        Err("original sample failed"),
        7,
        Ok(0x1000),
        Err("original clocks failed"),
    );
    assert_eq!(value.weather, [Some(-3), Some(-5)]);
    assert_eq!(value.night_factor, None);
    assert_eq!(value.target_region, None);
    let failed = captured.modifiers(&memory, 0, 0x3000);
    memory.array(0x3000, 0, 0);
    assert_eq!(captured.modifiers(&memory, 0, 0x3000), failed);
    assert_eq!(memory.reads.borrow()[&0x3000], 1);
}

#[test]
fn capacity_refusal_keeps_the_full_original_header_demand_for_one_growth() {
    let mut memory = Sparse::default();
    memory.array(0x1000, 0x2000, 3);
    for index in 0..3 {
        memory.row(0x2000 + index * 16, 0x6f, index as i64);
    }
    memory.metadata(0x6f);
    let mut captured = Capture::try_new(1, 0).unwrap();
    assert!(captured.modifiers(&memory, 0, 0x1000).is_err());
    assert_eq!(captured.required(), (1, 3));
    assert!(!memory.reads.borrow().contains_key(&0x2000));
    captured.retire();
    let required = captured.required();
    captured.try_grow(required.0, required.1).unwrap();
    assert_eq!(
        captured.modifiers(&memory, 0, 0x1000).unwrap().direct,
        [Some(3), Some(0)]
    );
}

#[test]
fn weather_detection_and_air_keys_share_one_original_modifier_header() {
    let mut memory = Sparse::default();
    memory.array(0x1058, 0x2000, 3);
    memory.row(0x2000, 0x2a, -7);
    memory.row(0x2010, 0x6f, -3);
    memory.row(0x2020, 0x70, -5);
    for key in [0x2a, 0x6f, 0x70] {
        memory.metadata(key);
    }
    let mut captured = Capture::try_new(1, 3).unwrap();
    let (environment, detection) = captured.environment_and_detection(
        &memory,
        0,
        Err("original sample failed"),
        7,
        Ok(0x1000),
        Err("original clocks failed"),
    );
    assert_eq!(environment.weather, [Some(-3), Some(-5)]);
    assert_eq!(detection, Ok(Some(-7)));
    assert_eq!(memory.reads.borrow()[&0x1058], 1);
    let (environment, detection) = captured.environment_and_detection(
        &memory,
        0,
        Ok(0),
        7,
        Ok(0),
        Err("original clocks failed"),
    );
    assert_eq!(environment.weather, [None; 2]);
    assert_eq!(detection, Ok(None));
}

#[test]
fn original_strict_weather_survives_extension_capacity_without_metadata_or_reread() {
    let mut memory = Sparse::default();
    let keys = crate::naval_mission_inputs::TASK_FORCE_KEYS;
    memory.array(0x1058, 0x2000, keys.len() as u32);
    for (index, key) in keys.into_iter().enumerate() {
        memory.row(0x2000 + index * 16, key, -7 - index as i64);
    }
    let mut captured = Capture::try_new(1, 0).unwrap();
    let (environment, detection, naval) = captured.environment_detection_and_naval(
        &memory,
        0,
        Err("original sample failed"),
        7,
        Ok(0x1000),
        Err("original clocks failed"),
    );
    assert_eq!(environment.weather, [None; 2]);
    assert!(detection.is_err());
    assert_eq!(
        naval.unwrap(),
        std::array::from_fn(|index| Value {
            raw: -7 - index as i64,
            present: true
        })
    );
    assert_eq!(captured.required(), (1, 4));
    memory.row(0x2000, keys[0], 999);
    assert_eq!(
        captured
            .environment_detection_and_naval(
                &memory,
                0,
                Err("original sample failed"),
                7,
                Ok(0x1000),
                Err("original clocks failed")
            )
            .2,
        naval
    );
    assert_eq!(memory.reads.borrow()[&0x1058], 1);
    assert_eq!(memory.reads.borrow()[&0x2000], 1);
    assert!(
        !memory
            .reads
            .borrow()
            .contains_key(&modifier_definition::TABLE_RVA)
    );
    assert_eq!(captured.strict_weather(&memory, 0, Ok(0x1000)), naval);
    assert_eq!(memory.reads.borrow()[&0x1058], 1);

    let mut unprepared = Capture::vacant();
    assert!(
        unprepared
            .environment_detection_and_naval(
                &memory,
                0,
                Err("original sample failed"),
                7,
                Ok(0x1000),
                Err("original clocks failed")
            )
            .2
            .is_err()
    );
    assert_eq!(memory.reads.borrow()[&0x1058], 1);
    assert!(unprepared.strict_weather(&memory, 0, Ok(0x1000)).is_ok());
    assert_eq!(memory.reads.borrow()[&0x1058], 2);
}

#[test]
fn strict_weather_keeps_full_suffix_failures_independent_of_native_lookup() {
    let mut memory = Sparse::default();
    memory.array(0x1058, 0x2000, 3);
    memory.row(0x2000, 0x2a, -7);
    memory.row(0x2010, 0x6f, -3);
    memory.row(0x2020, 0x70, -5);
    for key in [0x2a, 0x6f, 0x70] {
        memory.metadata(key);
    }
    // The original full16 suffix attempt fails on unused padding. The
    // successful earlier attack lookup remains independent of that fault.
    memory.bytes.remove(&0x2024);
    let mut captured = Capture::try_new(1, 3).unwrap();
    let (environment, _, naval) = captured.environment_detection_and_naval(
        &memory,
        0,
        Err("original sample failed"),
        7,
        Ok(0x1000),
        Err("original clocks failed"),
    );
    assert_eq!(environment.weather[0], Some(-3));
    assert_eq!(naval, Err("modifier accumulator entry unreadable"));
    memory.row(0x2020, 0x70, -5);
    assert_eq!(
        captured
            .environment_detection_and_naval(
                &memory,
                0,
                Err("original sample failed"),
                7,
                Ok(0x1000),
                Err("original clocks failed")
            )
            .2,
        naval
    );
    assert_eq!(memory.reads.borrow()[&0x2020], 1);
}

#[test]
fn assigned_ace_wing_borrows_original_planes_without_reading_the_wing_again() {
    let memory = Sparse::default();
    let reference = 9_u64 << 32 | u64::from(crate::cair_wing::TYPE);
    let original = WingIdentity {
        mission_pointer: 0x1090,
        wing: reference,
        planes: -3,
        ..Default::default()
    };
    assert_eq!(
        assigned_count(&memory, 0, &[original], reference, Ok(Some(0x1010))),
        Ok(Some(-3))
    );
    assert_eq!(
        assigned_count(&memory, 0, &[original], reference + 1, Ok(Some(0x1010))),
        Err("transport ace assignment original wing handle mismatch")
    );
    assert_eq!(
        assigned_count(
            &memory,
            0,
            &[original],
            reference,
            Err("original resolver failed")
        ),
        Err("original resolver failed")
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn normalization_demand_excludes_allocator_spare_capacity() {
    let mut capture = Capture::try_new(0, 0).unwrap();
    capture.nodes.reserve(30_000);
    capture.entries.reserve(30_000);
    capture.enable_worker_growth();
    capture.ensure_nodes(5_001).unwrap();
    capture.ensure_entries(5_001).unwrap();
    for _ in 0..20 {
        let demand = capture.capacity();
        assert_eq!(demand, (5_001, 5_001));
        capture.try_grow(demand.0, demand.1).unwrap();
    }
}
